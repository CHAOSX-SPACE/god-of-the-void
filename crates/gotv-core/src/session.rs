//! Sesiones persistentes (P4): la conversación sobrevive entre órdenes y entre arranques.
//!
//! **Rust puro** (orden del Portador, 2026-11-17): cada sesión son dos archivos en `.gotv/sesiones/`, en el
//! formato binario propio de [`crate::programar::codec`]:
//! - `<id>.cab` · la cabecera: fecha, modelo, título, cuántos mensajes y el [`Estado`] de la sesión (lo que el
//!   agente debe recordar además de los mensajes: hoy, las herramientas ya cargadas, que son parte del
//!   PREFIJO del prompt; si al reabrir cambian, la memoria rápida de la conversación deja de valer entera);
//! - `<id>.msj` · los mensajes, campo a campo (las llamadas a herramientas, como árbol binario).
//!
//! Listar lee sólo las cabeceras. Se escribe de forma atómica (temporal + renombrar) y los ids nuevos se dan
//! bajo un candado (`flock`): la app y la CLI pueden abrir el mismo proyecto a la vez.
//!
//! **Lo de antes:** las conversaciones que guardaba GOTV en `.gotv/gotv.db` (SQLite + JSON) se pasan UNA vez,
//! leyéndola en sólo lectura, con sus mismos ids; `gotv.db` queda intacta como respaldo (L6: no se borra).
//! Una sesión escrita por un GOTV más nuevo (versión de formato mayor) no se pisa: se avisa y no se guarda.
use crate::programar::codec::{self, Candado, Escritor, Lector};
use anyhow::{bail, Context, Result};
use gotv_engine::Message;
use std::path::{Path, PathBuf};

const CAB: &[u8; 4] = b"SCAB";
const MSJ: &[u8; 4] = b"SMSJ";
/// La versión del formato de la CABECERA que este GOTV escribe y sabe leer. F8.1 (migración 1 de F0.6):
/// la 2 añade el padre de un chat bifurcado (`parent`) y la marca de papelera (`deleted_at`); una
/// cabecera 1 se lee con esos campos vacíos y pasa a 2 la próxima vez que se guarda.
pub const VERSION: u8 = 2;
/// La de los MENSAJES (F8.1 no la cambió: un GOTV anterior sigue leyéndolos).
const VERSION_MSJ: u8 = 1;
/// Las que se listan (las más nuevas).
const LISTA_MAX: usize = 50;
/// F8.1: el título automático es la primera orden recortada a 60 caracteres.
const TITULO_MAX: usize = 60;
/// Días en la papelera antes de que «vaciar» borre un chat de verdad.
pub const DIAS_PAPELERA: i64 = 30;

/// F8.1: el formato de [`Session::export`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formato {
    Markdown,
    Json,
}

impl Formato {
    pub fn parse(s: &str) -> Option<Formato> {
        match s.trim().to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Formato::Markdown),
            "json" => Some(Formato::Json),
            _ => None,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Formato::Markdown => "md",
            Formato::Json => "json",
        }
    }
}

/// Lo que la sesión recuerda además de los mensajes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Estado {
    /// las herramientas diferidas que el modelo ya cargó en esta sesión
    pub tools_loaded: Vec<String>,
}

pub struct Session {
    dir: PathBuf,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub id: i64,
    pub created: String,
    pub title: String,
    pub messages: usize,
    /// F8.1: el chat del que se bifurcó y el índice del último mensaje que copió.
    pub parent: Option<(i64, u32)>,
    /// F8.1: cuándo fue a la papelera (segundos Unix); `None` = vivo.
    pub deleted_at: Option<i64>,
}

/// La cabecera de una sesión.
#[derive(Debug, Clone, PartialEq)]
struct Cabecera {
    id: i64,
    created: String,
    model: String,
    title: String,
    mensajes: u32,
    estado: Estado,
    /// v2: el chat padre y el índice del último mensaje copiado
    parent: Option<(i64, u32)>,
    /// v2: la marca de papelera (segundos Unix)
    deleted_at: Option<i64>,
}

impl Cabecera {
    fn bytes(&self) -> Vec<u8> {
        let mut e = Escritor::nuevo(CAB, VERSION);
        e.i64(self.id);
        e.texto(&self.created);
        e.texto(&self.model);
        e.texto(&self.title);
        e.u32(self.mensajes);
        e.textos(&self.estado.tools_loaded);
        e.opt_i64(self.parent.map(|p| p.0));
        e.opt_i64(self.parent.map(|p| p.1 as i64));
        e.opt_i64(self.deleted_at);
        e.bytes()
    }

    fn de_bytes(b: &[u8]) -> Result<Cabecera> {
        let (mut l, v) = Lector::nuevo(b, CAB, VERSION)?;
        let mut c = Cabecera { id: l.i64()?, created: l.texto()?, model: l.texto()?, title: l.texto()?, mensajes: l.u32()?, estado: Estado { tools_loaded: l.textos()? }, parent: None, deleted_at: None };
        // migración 1 (F8.1): la versión 1 no traía padre ni papelera
        if v >= 2 {
            let (p, pm) = (l.opt_i64()?, l.opt_i64()?);
            c.parent = p.map(|p| (p, pm.unwrap_or(0).clamp(0, u32::MAX as i64) as u32));
            c.deleted_at = l.opt_i64()?;
        }
        Ok(c)
    }

    fn info(self) -> SessionInfo {
        SessionInfo { id: self.id, created: self.created, title: self.title, messages: self.mensajes as usize, parent: self.parent, deleted_at: self.deleted_at }
    }
}

/// ¿Es un mensaje que ESCRIBIÓ el Portador (no un aviso del programa ni el anexo interno de adjuntos)?
fn es_del_portador(m: &Message) -> bool {
    m.role == "user" && !m.es_aviso() && !crate::mensajes::es_adjuntos_pedidos(m)
}

/// F8.1: el título automático: la primera orden que escribió el Portador (sin anexos ni avisos), en una
/// línea y recortada a [`TITULO_MAX`] caracteres. Lo cambia el Portador ([`Session::rename`]), no el modelo.
pub fn titulo_de(history: &[Message]) -> Option<String> {
    let t = crate::mensajes::visible(history.iter().find(|m| es_del_portador(m))?).split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return None;
    }
    Some(if t.chars().count() > TITULO_MAX { format!("{}…", t.chars().take(TITULO_MAX - 1).collect::<String>().trim_end()) } else { t })
}

fn ahora_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn escribir_mensaje(e: &mut Escritor, m: &Message) {
    e.texto(&m.role);
    e.texto(&m.content);
    e.bool(m.tool_calls.is_some());
    if let Some(cs) = &m.tool_calls {
        e.u32(cs.len() as u32);
        for c in cs {
            e.valor(c);
        }
    }
    e.opt_texto(m.tool_call_id.as_deref());
    e.opt_texto(m.name.as_deref());
    e.textos(&m.images);
    e.textos(&m.audio);
    e.opt_valor(m.blocks.as_ref());
    e.opt_texto(m.visible.as_deref());
}

fn leer_mensaje(l: &mut Lector) -> Result<Message> {
    let role = l.texto()?;
    let content = l.texto()?;
    let tool_calls = if l.bool()? {
        let n = l.u32()? as usize;
        Some((0..n).map(|_| l.valor()).collect::<Result<Vec<_>>>()?)
    } else {
        None
    };
    Ok(Message { role, content, tool_calls, tool_call_id: l.opt_texto()?, name: l.opt_texto()?, images: l.textos()?, audio: l.textos()?, blocks: l.opt_valor()?, visible: l.opt_texto()? })
}

fn mensajes_bytes(h: &[Message]) -> Vec<u8> {
    let mut e = Escritor::nuevo(MSJ, VERSION_MSJ);
    e.u32(h.len() as u32);
    for m in h {
        escribir_mensaje(&mut e, m);
    }
    e.bytes()
}

fn mensajes_de_bytes(b: &[u8]) -> Result<Vec<Message>> {
    let (mut l, _) = Lector::nuevo(b, MSJ, VERSION_MSJ)?;
    let n = l.u32()? as usize;
    (0..n).map(|_| leer_mensaje(&mut l)).collect()
}

fn carpeta(root: &Path) -> PathBuf {
    root.join(".gotv/sesiones")
}

fn ahora_local() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Los ids que hay en la carpeta (por los nombres `<id>.cab`).
fn ids(dir: &Path) -> Vec<i64> {
    let mut v: Vec<i64> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".cab")).and_then(|n| n.parse().ok()))
        .collect();
    v.sort_unstable();
    v
}

fn ruta_cab(dir: &Path, id: i64) -> PathBuf {
    dir.join(format!("{id}.cab"))
}

fn ruta_msj(dir: &Path, id: i64) -> PathBuf {
    dir.join(format!("{id}.msj"))
}

/// Lo que la COMPACTACIÓN sacó de la conversación: el modelo ya no lo ve (le queda el resumen), pero el Portador
/// sí debe seguir viéndolo al reabrir el chat. Mismo formato que los mensajes (`SMSJ`), sólo crece.
fn ruta_arc(dir: &Path, id: i64) -> PathBuf {
    dir.join(format!("{id}.arc"))
}

fn leer_cab(dir: &Path, id: i64) -> Result<Cabecera> {
    let b = std::fs::read(ruta_cab(dir, id)).with_context(|| crate::tr!("no pude leer la sesión #{id}", "could not read session #{id}"))?;
    Cabecera::de_bytes(&b)
}

/// Prepara la carpeta: la crea y, si el proyecto tiene conversaciones de antes en `gotv.db`, las pasa una vez.
fn preparar(root: &Path) -> Result<PathBuf> {
    let dir = carpeta(root);
    std::fs::create_dir_all(&dir).with_context(|| crate::tr!("no pude crear {}", "could not create {}", dir.display()))?;
    let marca = dir.join("migrada-de-gotv-db");
    let vieja = root.join(".gotv/gotv.db");
    if vieja.is_file() && !marca.exists() {
        let _c = Candado::esperar(&dir.join(".candado"))?;
        if !marca.exists() {
            migrar_de_sqlite(&vieja, &dir)?;
            std::fs::write(&marca, b"")?;
        }
    }
    Ok(dir)
}

/// Pasa las conversaciones de `gotv.db` (SQLite + JSON, de antes) al formato propio, con sus mismos ids. La
/// base se abre en SÓLO LECTURA y no se toca. Una sesión que ya exista en el formato nuevo no se pisa.
fn migrar_de_sqlite(db: &Path, dir: &Path) -> Result<usize> {
    use rusqlite::{params, Connection, OpenFlags};
    let c = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).with_context(|| crate::tr!("no pude leer {}", "could not read {}", db.display()))?;
    let hay = |t: &str| -> bool { c.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)", params![t], |r| r.get(0)).unwrap_or(false) };
    if !hay("sessions") || !hay("messages") {
        return Ok(0);
    }
    let con_estado = hay("session_state");
    let mut st = c.prepare("SELECT id, created, model, title FROM sessions ORDER BY id")?;
    let sesiones: Vec<(i64, String, String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.filter_map(|x| x.ok()).collect();
    let mut n = 0;
    for (id, created, model, title) in sesiones {
        if ruta_cab(dir, id).exists() {
            continue;
        }
        let mut st = c.prepare("SELECT json FROM messages WHERE session_id = ?1 ORDER BY seq")?;
        // lo de antes venía en JSON: se lee aquí, UNA vez, y desde ya se guarda en Rust
        let mensajes: Vec<Message> = st.query_map(params![id], |r| r.get::<_, String>(0))?.filter_map(|j| j.ok()).filter_map(|j| serde_json::from_str(&j).ok()).collect();
        let tools_loaded: Vec<String> = if con_estado {
            c.query_row("SELECT json FROM session_state WHERE session_id = ?1", params![id], |r| r.get::<_, String>(0))
                .ok()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
                .and_then(|v| v["tools_loaded"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()))
                .unwrap_or_default()
        } else {
            vec![]
        };
        codec::guardar(&ruta_msj(dir, id), &mensajes_bytes(&mensajes))?;
        let cab = Cabecera { id, created, model, title, mensajes: mensajes.len() as u32, estado: Estado { tools_loaded }, parent: None, deleted_at: None };
        codec::guardar(&ruta_cab(dir, id), &cab.bytes())?;
        n += 1;
    }
    Ok(n)
}

/// F0.6: el aviso para la ventana si alguna sesión de este proyecto la escribió un GOTV más nuevo (entonces
/// se ve en la lista pero no se guardan cambios en ella).
pub fn aviso_de_version(root: &Path) -> Option<String> {
    let dir = carpeta(root);
    let v = ids(&dir).into_iter().filter_map(|id| std::fs::read(ruta_cab(&dir, id)).ok()).filter_map(|b| codec::version_de(&b, CAB)).max()?;
    (v > VERSION).then(|| {
        crate::tr!(
            "Hay conversaciones de este proyecto que guardó un GOTV más nuevo (formato {v}; éste conoce hasta el {VERSION}). No se tocan: actualiza GOTV.",
            "Some of this project's conversations were saved by a newer GOTV (format {v}; this one knows up to {VERSION}). They are not touched: update GOTV."
        )
    })
}

impl Session {
    /// Abre una sesión nueva en este proyecto.
    pub fn open_new(root: &Path, model: &str) -> Result<Session> {
        let dir = preparar(root)?;
        let _c = Candado::esperar(&dir.join(".candado"))?;
        let id = ids(&dir).last().copied().unwrap_or(0) + 1;
        let cab = Cabecera { id, created: ahora_local(), model: model.to_string(), title: String::new(), mensajes: 0, estado: Estado::default(), parent: None, deleted_at: None };
        codec::guardar(&ruta_cab(&dir, id), &cab.bytes())?;
        Ok(Session { dir, id })
    }

    /// F8.1: el asa de un chat que EXISTE en este proyecto (sin leer sus mensajes). Un id que no es de este
    /// proyecto falla.
    pub fn de(root: &Path, id: i64) -> Result<Session> {
        let dir = carpeta(root);
        leer_cab(&dir, id)?;
        Ok(Session { dir, id })
    }

    /// F8.1: abre un chat concreto de este proyecto, con su historial. Uno de la papelera no se abre (antes
    /// se restaura) y un id de otro proyecto falla.
    pub fn open(root: &Path, id: i64) -> Result<(Session, Vec<Message>)> {
        let dir = preparar(root)?;
        let cab = leer_cab(&dir, id)?;
        if cab.deleted_at.is_some() {
            bail!("{}", crate::tr!("el chat #{id} está en la papelera: restáuralo para abrirlo", "chat #{id} is in the trash: restore it to open it"));
        }
        Ok((Session { dir, id }, leer_mensajes(&carpeta(root), id, cab.mensajes)?))
    }

    /// F8.1: cambia el título (lo pone el Portador; el modelo nunca). Una línea, hasta 120 caracteres.
    pub fn rename(&self, titulo: &str) -> Result<()> {
        let t = titulo.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            bail!("{}", crate::i18n::t("el título no puede quedar vacío", "the title cannot be empty"));
        }
        let mut cab = self.cabecera_escribible()?;
        cab.title = t.chars().take(120).collect();
        codec::guardar(&ruta_cab(&self.dir, self.id), &cab.bytes())
    }

    /// F8.1: un chat NUEVO con los mensajes `0..=hasta` del chat `id` y su estado; el nuevo apunta a su padre,
    /// que queda intacto. No se corta en medio de una llamada a herramientas (el motor la rechazaría sin su
    /// resultado). El nuevo NO hereda la memoria rápida (es por conversación): su primera orden relee todo.
    pub fn fork(root: &Path, id: i64, hasta: usize) -> Result<Session> {
        let (padre, h) = Session::open(root, id)?;
        let Some(ultimo) = h.get(hasta) else {
            bail!("{}", crate::tr!("el chat #{id} tiene {} mensajes: no hay un mensaje {hasta}", "chat #{id} has {} messages: there is no message {hasta}", h.len()));
        };
        if ultimo.tool_calls.is_some() || ultimo.role == "tool" {
            bail!("{}", crate::i18n::t("no se bifurca en medio de una llamada a herramientas: elige un mensaje tuyo o una respuesta", "cannot branch in the middle of a tool call: pick one of your messages or an answer"));
        }
        let cab_padre = leer_cab(&padre.dir, id)?;
        let dir = padre.dir;
        let _c = Candado::esperar(&dir.join(".candado"))?;
        let nuevo = ids(&dir).last().copied().unwrap_or(0) + 1;
        let copia = &h[..=hasta];
        codec::guardar(&ruta_msj(&dir, nuevo), &mensajes_bytes(copia))?;
        // lo que el padre compactó es la historia que precede a la rama: viaja con ella
        if let Ok(b) = std::fs::read(ruta_arc(&dir, id)) {
            codec::guardar(&ruta_arc(&dir, nuevo), &b)?;
        }
        let base = if cab_padre.title.is_empty() { titulo_de(&h).unwrap_or_default() } else { cab_padre.title.clone() };
        let cab = Cabecera {
            id: nuevo,
            created: ahora_local(),
            model: cab_padre.model,
            title: format!("↳ {base}").chars().take(120).collect(),
            mensajes: copia.len() as u32,
            estado: cab_padre.estado,
            parent: Some((id, hasta as u32)),
            deleted_at: None,
        };
        codec::guardar(&ruta_cab(&dir, nuevo), &cab.bytes())?;
        Ok(Session { dir, id: nuevo })
    }

    /// F8.1: el chat como texto para llevárselo: sólo lo que se ve en la ventana (tus mensajes sin anexos y
    /// las respuestas), nunca los mensajes internos (avisos del programa, llamadas a herramientas, sus
    /// resultados) ni imágenes o audios en base64 (de esos queda su ruta, o una marca si no tenían).
    pub fn export(root: &Path, id: i64, formato: Formato) -> Result<String> {
        let dir = preparar(root)?;
        let cab = leer_cab(&dir, id)?;
        let mut h = leer_mensajes(&dir, id, cab.mensajes)?;
        // la conversación ENTERA: primero lo que la compactación sacó de la vista del modelo (sin su memoria de trabajo)
        let mut todo = Session { dir: dir.clone(), id }.archivo();
        h.retain(|m| !crate::mensajes::es_memoria_de_trabajo(m));
        todo.append(&mut h);
        let h = todo;
        let partes = exportables(&h);
        let titulo = if cab.title.is_empty() { titulo_de(&h).unwrap_or_else(|| format!("#{id}")) } else { cab.title.clone() };
        Ok(match formato {
            Formato::Json => {
                let msgs: Vec<serde_json::Value> = partes.iter().map(|(rol, texto, medios)| serde_json::json!({"role": rol, "text": texto, "media": medios})).collect();
                let v = serde_json::json!({"id": id, "title": titulo, "created": cab.created, "model": cab.model, "parent": cab.parent.map(|p| serde_json::json!({"id": p.0, "message": p.1})), "messages": msgs});
                serde_json::to_string_pretty(&v)?
            }
            Formato::Markdown => {
                let mut s = format!("# {titulo}\n\n_{} · {}_\n", cab.created, cab.model);
                if let Some((p, m)) = cab.parent {
                    s.push_str(&crate::tr!("_bifurcado del chat #{p} en el mensaje {m}_\n", "_branched from chat #{p} at message {m}_\n"));
                }
                for (rol, texto, medios) in &partes {
                    let quien = if rol == "user" { crate::i18n::t("Tú", "You") } else { "GOTV" };
                    s.push_str(&format!("\n## {quien}\n\n{}\n", texto.trim()));
                    for m in medios {
                        s.push_str(&format!("\n- {m}\n"));
                    }
                }
                s
            }
        })
    }

    /// F8.1: manda el chat a la papelera. Sólo se MARCA: no se borra nada (lo borra «vaciar» pasados
    /// [`DIAS_PAPELERA`] días). No cambia la lista de los demás.
    pub fn trash(root: &Path, id: i64) -> Result<()> {
        Session::de(root, id)?.marcar_papelera(Some(ahora_unix()))
    }

    /// F8.1: lo saca de la papelera.
    pub fn restore(root: &Path, id: i64) -> Result<()> {
        Session::de(root, id)?.marcar_papelera(None)
    }

    fn marcar_papelera(&self, cuando: Option<i64>) -> Result<()> {
        let mut cab = self.cabecera_escribible()?;
        cab.deleted_at = cuando;
        codec::guardar(&ruta_cab(&self.dir, self.id), &cab.bytes())
    }

    /// F8.1: «vaciar la papelera»: borra DE VERDAD, por su id, los chats marcados hace más de `dias` días
    /// (sus dos archivos y su carpeta de documentos `.gotv/chats/<id>/`). Nada más se borra nunca.
    /// Devuelve cuántos borró.
    pub fn vaciar(root: &Path, dias: i64) -> Result<usize> {
        let dir = carpeta(root);
        if !dir.is_dir() {
            return Ok(0);
        }
        let _c = Candado::esperar(&dir.join(".candado"))?;
        let limite = ahora_unix() - dias * 86_400;
        let mut n = 0;
        for id in ids(&dir) {
            let Ok(cab) = leer_cab(&dir, id) else { continue };
            if cab.deleted_at.is_some_and(|t| t <= limite) {
                let _ = std::fs::remove_file(ruta_msj(&dir, id));
                let _ = std::fs::remove_file(ruta_arc(&dir, id));
                std::fs::remove_file(ruta_cab(&dir, id))?;
                let docs = crate::artefactos::chat_de(root, Some(id), "").dir().to_path_buf();
                if docs.is_dir() {
                    let _ = std::fs::remove_dir_all(docs);
                }
                n += 1;
            }
        }
        Ok(n)
    }

    /// F8.1: los chats de la papelera, del más reciente al más viejo.
    pub fn papelera(root: &Path) -> Result<Vec<SessionInfo>> {
        let dir = carpeta(root);
        let mut v: Vec<SessionInfo> = ids(&dir).into_iter().filter_map(|id| leer_cab(&dir, id).ok()).filter(|c| c.deleted_at.is_some()).map(Cabecera::info).collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.deleted_at));
        Ok(v)
    }

    /// F8.1: una PÁGINA de los chats vivos con mensajes (del más nuevo al más viejo): `cuantos` desde el
    /// puesto `desde`, y cuántos hay en total. Un proyecto con cientos de chats no se manda de golpe.
    pub fn page(root: &Path, desde: usize, cuantos: usize) -> Result<(Vec<SessionInfo>, usize)> {
        if !carpeta(root).is_dir() && !root.join(".gotv/gotv.db").is_file() {
            return Ok((vec![], 0));
        }
        let dir = preparar(root)?;
        let vivos: Vec<Cabecera> = ids(&dir).into_iter().rev().filter_map(|id| leer_cab(&dir, id).ok()).filter(|c| c.mensajes > 0 && c.deleted_at.is_none()).collect();
        let total = vivos.len();
        Ok((vivos.into_iter().skip(desde).take(cuantos).map(Cabecera::info).collect(), total))
    }

    /// La sesión más reciente CON mensajes (excepto `exclude`), con su historial.
    pub fn resume_last(root: &Path, exclude: Option<i64>) -> Result<Option<(Session, Vec<Message>)>> {
        if !carpeta(root).is_dir() && !root.join(".gotv/gotv.db").is_file() {
            return Ok(None);
        }
        let dir = preparar(root)?;
        for id in ids(&dir).into_iter().rev() {
            if Some(id) == exclude {
                continue;
            }
            let Ok(cab) = leer_cab(&dir, id) else { continue };
            // F8.1: lo de la papelera no se retoma solo
            if cab.mensajes == 0 || cab.deleted_at.is_some() {
                continue;
            }
            let Ok(b) = std::fs::read(ruta_msj(&dir, id)) else { continue };
            let Ok(h) = mensajes_de_bytes(&b) else { continue };
            return Ok(Some((Session { dir, id }, h)));
        }
        Ok(None)
    }

    /// El estado extra de la sesión (vacío si no hay).
    pub fn load_state(&self) -> Estado {
        leer_cab(&self.dir, self.id).map(|c| c.estado).unwrap_or_default()
    }

    /// Guarda el estado extra de la sesión (lo reemplaza).
    pub fn save_state(&self, estado: &Estado) -> Result<()> {
        let mut cab = self.cabecera_escribible()?;
        cab.estado = estado.clone();
        codec::guardar(&ruta_cab(&self.dir, self.id), &cab.bytes())
    }

    /// Guarda el historial completo de la sesión (atómico: primero los mensajes, luego la cabecera).
    pub fn save(&self, history: &[Message]) -> Result<()> {
        let mut cab = self.cabecera_escribible()?;
        codec::guardar(&ruta_msj(&self.dir, self.id), &mensajes_bytes(history))?;
        cab.mensajes = history.len() as u32;
        if cab.title.is_empty() {
            if let Some(t) = titulo_de(history) {
                cab.title = t;
            }
        }
        codec::guardar(&ruta_cab(&self.dir, self.id), &cab.bytes())
    }

    /// Archiva lo que la compactación saca de la conversación (se AÑADE a lo ya archivado). Medido 2026-11-18 (el
    /// Portador): al reabrir la app, lo anterior a una compactación había desaparecido del chat, porque `save`
    /// guarda sólo lo que el modelo sigue viendo.
    pub fn archivar(&self, viejos: &[Message]) -> Result<()> {
        if viejos.is_empty() {
            return Ok(());
        }
        let mut todo = self.archivo();
        todo.extend_from_slice(viejos);
        codec::guardar(&ruta_arc(&self.dir, self.id), &mensajes_bytes(&todo))
    }

    /// Lo archivado por la compactación, en orden (vacío si nunca se compactó).
    pub fn archivo(&self) -> Vec<Message> {
        std::fs::read(ruta_arc(&self.dir, self.id)).ok().and_then(|b| mensajes_de_bytes(&b).ok()).unwrap_or_default()
    }

    /// La cabecera, si este GOTV puede escribirla (una de un GOTV más nuevo no se pisa).
    fn cabecera_escribible(&self) -> Result<Cabecera> {
        let ruta = ruta_cab(&self.dir, self.id);
        let b = std::fs::read(&ruta).with_context(|| crate::tr!("no pude leer la sesión #{}", "could not read session #{}", self.id))?;
        if codec::version_de(&b, CAB).is_some_and(|v| v > VERSION) {
            bail!("{}", crate::tr!("la sesión #{} la guardó un GOTV más nuevo: no la piso", "session #{} was saved by a newer GOTV: I will not overwrite it", self.id));
        }
        Cabecera::de_bytes(&b)
    }

    /// Sesiones del proyecto con mensajes, de la más nueva a la más vieja (las 50 últimas; sin las de la
    /// papelera).
    pub fn list(root: &Path) -> Result<Vec<SessionInfo>> {
        Ok(Session::page(root, 0, LISTA_MAX)?.0)
    }
}

/// Los mensajes de un chat (vacío si la cabecera dice que no tiene).
fn leer_mensajes(dir: &Path, id: i64, n: u32) -> Result<Vec<Message>> {
    if n == 0 && !ruta_msj(dir, id).is_file() {
        return Ok(vec![]);
    }
    let b = std::fs::read(ruta_msj(dir, id)).with_context(|| crate::tr!("no pude leer los mensajes del chat #{id}", "could not read the messages of chat #{id}"))?;
    mensajes_de_bytes(&b)
}

/// Lo exportable de un historial: (rol, texto, medios) de lo que se ve en la ventana.
fn exportables(h: &[Message]) -> Vec<(String, String, Vec<String>)> {
    // de un medio adjunto queda su ruta; uno incrustado (`data:`) se dice, nunca se copia
    let medio = |x: &String, imagen: bool| -> String {
        if x.starts_with("data:") {
            if imagen {
                crate::i18n::t("(imagen adjunta, incrustada: no se exporta)", "(attached image, embedded: not exported)").to_string()
            } else {
                crate::i18n::t("(audio adjunto, incrustado: no se exporta)", "(attached audio, embedded: not exported)").to_string()
            }
        } else {
            x.clone()
        }
    };
    let mut out = Vec::new();
    for m in h {
        if es_del_portador(m) {
            let t = crate::mensajes::visible(m).to_string();
            let medios: Vec<String> = m.images.iter().map(|x| medio(x, true)).chain(m.audio.iter().map(|x| medio(x, false))).collect();
            if !t.trim().is_empty() || !medios.is_empty() {
                out.push(("user".to_string(), t, medios));
            }
        } else if m.role == "assistant" && m.tool_calls.is_none() && !m.content.trim().is_empty() {
            let t = crate::mensajes::sin_ubicaciones(m.content.split("\n\n(Skills").next().unwrap_or(&m.content));
            if !t.trim().is_empty() {
                out.push(("assistant".to_string(), t, vec![]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gotv_engine::ToolCall;

    fn root(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-ses-{tag}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn el_estado_de_la_sesion_sobrevive_y_es_de_cada_sesion() {
        let r = root("estado");
        let a = Session::open_new(&r, "m").unwrap();
        let b = Session::open_new(&r, "m").unwrap();
        assert_eq!(a.load_state(), Estado::default(), "sin estado: vacío");
        a.save_state(&Estado { tools_loaded: vec!["browser".into(), "pagina".into()] }).unwrap();
        a.save_state(&Estado { tools_loaded: vec!["browser".into(), "pagina".into(), "lsp".into()] }).unwrap();
        // se reabre (otro valor) y se lee lo último; la otra sesión no lo ve
        let a2 = Session { dir: carpeta(&r), id: a.id };
        assert_eq!(a2.load_state().tools_loaded.len(), 3);
        assert_eq!(b.load_state(), Estado::default());
        // guardar mensajes no pisa el estado (los dos viven en la cabecera)
        a.save(&[Message::user("hola")]).unwrap();
        assert_eq!(a2.load_state().tools_loaded.len(), 3);
    }

    #[test]
    fn guarda_y_retoma_con_llamadas_campo_a_campo() {
        let r = root("ida");
        let s = Session::open_new(&r, "m").unwrap();
        let call = ToolCall { id: "c0".into(), name: "read_file".into(), arguments: serde_json::json!({"path": "a", "n": 3}) };
        let mut con_todo = Message::user_with_media("mira ⏰ ñ", vec!["data:image/png;base64,AAA".into(), "data:audio/wav;base64,BBB".into()]);
        con_todo.visible = Some("mira".into());
        con_todo.blocks = Some(serde_json::json!([{"type": "thinking", "thinking": "x"}]));
        let h = vec![
            Message::system("s"),
            Message::user("lee a"),
            Message::assistant_calls("", std::slice::from_ref(&call)),
            Message::tool_result(&call, "contenido"),
            Message::aviso("un aviso del programa"),
            con_todo,
            Message::assistant("listo"),
        ];
        s.save(&h).unwrap();
        let (s2, h2) = Session::resume_last(&r, None).unwrap().unwrap();
        assert_eq!(s2.id, s.id);
        assert_eq!(h2.len(), h.len());
        for (a, b) in h2.iter().zip(&h) {
            assert_eq!(
                (&a.role, &a.content, &a.tool_calls, &a.tool_call_id, &a.name, &a.images, &a.audio, &a.blocks, &a.visible),
                (&b.role, &b.content, &b.tool_calls, &b.tool_call_id, &b.name, &b.images, &b.audio, &b.blocks, &b.visible),
                "cada mensaje vuelve idéntico, campo a campo"
            );
        }
        assert!(h2[4].es_aviso());
        let l = Session::list(&r).unwrap();
        assert_eq!((l[0].title.as_str(), l[0].messages), ("lee a", 7));
        // el formato es el propio: ni rastro de JSON en los archivos
        let crudo = std::fs::read(ruta_msj(&carpeta(&r), s.id)).unwrap();
        assert!(crudo.starts_with(b"GOTVSMSJ"));
        assert!(!String::from_utf8_lossy(&crudo).contains("\"role\""));
    }

    #[test]
    fn resume_salta_la_actual_y_las_vacias() {
        let r = root("salta");
        let vieja = Session::open_new(&r, "m").unwrap();
        vieja.save(&[Message::system("s"), Message::user("vieja")]).unwrap();
        let actual = Session::open_new(&r, "m").unwrap(); // vacía
        let (s, h) = Session::resume_last(&r, Some(actual.id)).unwrap().unwrap();
        assert_eq!(s.id, vieja.id);
        assert_eq!(h[1].content, "vieja");
        assert_eq!(Session::list(&r).unwrap().len(), 1, "la vacía no se lista");
        assert!(Session::resume_last(&root("sin-nada"), None).unwrap().is_none());
    }

    /// La app y la CLI abren el mismo proyecto a la vez: los ids nuevos no se repiten.
    #[test]
    fn ids_sin_repetir_aunque_abran_dos_a_la_vez() {
        let r = root("ids");
        let hilos: Vec<_> = (0..4)
            .map(|_| {
                let r = r.to_path_buf();
                std::thread::spawn(move || (0..10).map(|_| Session::open_new(&r, "m").unwrap().id).collect::<Vec<_>>())
            })
            .collect();
        let mut todos: Vec<i64> = hilos.into_iter().flat_map(|h| h.join().unwrap()).collect();
        todos.sort_unstable();
        assert_eq!(todos, (1..=40).collect::<Vec<_>>());
    }

    /// La base TAL COMO la dejaba GOTV antes (SQLite + JSON), con datos.
    fn base_de_antes(r: &Path) -> PathBuf {
        use rusqlite::{params, Connection};
        let db = r.join(".gotv/gotv.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let c = Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, created TEXT NOT NULL DEFAULT (datetime('now','localtime')), model TEXT NOT NULL DEFAULT '', title TEXT NOT NULL DEFAULT '');
             CREATE TABLE messages (session_id INTEGER NOT NULL, seq INTEGER NOT NULL, json TEXT NOT NULL, PRIMARY KEY (session_id, seq));
             CREATE TABLE session_state (session_id INTEGER PRIMARY KEY, json TEXT NOT NULL);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        c.execute("INSERT INTO sessions (id, created, model, title) VALUES (3, '2026-11-10 09:00:00', 'm', 'vieja')", []).unwrap();
        c.execute("INSERT INTO sessions (id, model, title) VALUES (7, 'm', '')", []).unwrap(); // vacía
        for i in 0..46 {
            c.execute("INSERT INTO messages (session_id, seq, json) VALUES (3, ?1, ?2)", params![i, serde_json::to_string(&Message::user(format!("m{i}"))).unwrap()]).unwrap();
        }
        c.execute("INSERT INTO session_state (session_id, json) VALUES (3, ?1)", params![r#"{"tools_loaded":["browser","lsp"]}"#]).unwrap();
        db
    }

    /// Lo de antes pasa al formato propio sin perder un mensaje, con sus ids, una sola vez, y `gotv.db`
    /// queda intacta (respaldo); las sesiones nuevas siguen la numeración.
    #[test]
    fn las_conversaciones_de_antes_pasan_una_vez_sin_perder_nada() {
        let r = root("migra");
        let db = base_de_antes(&r);
        let antes = std::fs::read(&db).unwrap();
        let (s, h) = Session::resume_last(&r, None).unwrap().unwrap();
        assert_eq!((s.id, h.len()), (3, 46), "los 46 mensajes siguen, con su id");
        assert_eq!(h[45].content, "m45");
        assert_eq!(s.load_state().tools_loaded, ["browser", "lsp"]);
        let l = Session::list(&r).unwrap();
        assert_eq!((l.len(), l[0].title.as_str(), l[0].created.as_str()), (1, "vieja", "2026-11-10 09:00:00"));
        assert_eq!(std::fs::read(&db).unwrap(), antes, "gotv.db no se tocó");
        // una sesión nueva sigue después de la última de antes (la 7, vacía)
        assert_eq!(Session::open_new(&r, "m").unwrap().id, 8);
        // otra apertura no vuelve a migrar: lo nuevo no se pisa con lo de antes
        s.save(&[Message::user("cambiada")]).unwrap();
        Session::list(&r).unwrap();
        assert_eq!(Session::resume_last(&r, None).unwrap().unwrap().1.len(), 1);
    }

    /// Una sesión de un GOTV MÁS NUEVO no se pisa, y la ventana lo puede decir.
    #[test]
    fn una_sesion_mas_nueva_no_se_pisa() {
        let r = root("mas-nueva");
        let s = Session::open_new(&r, "m").unwrap();
        s.save(&[Message::user("x")]).unwrap();
        let ruta = ruta_cab(&carpeta(&r), s.id);
        let mut b = std::fs::read(&ruta).unwrap();
        b[8] = VERSION + 3;
        std::fs::write(&ruta, &b).unwrap();
        assert!(s.save(&[Message::user("y")]).is_err(), "no se escribe");
        assert!(s.save_state(&Estado::default()).is_err());
        assert_eq!(std::fs::read(&ruta).unwrap(), b, "ni se toca");
        assert!(aviso_de_version(&r).is_some_and(|a| a.contains("GOTV")));
        assert!(aviso_de_version(&root("sin-sesiones")).is_none());
    }

    /// Un chat de `n` mensajes alternos (tú / GOTV) con estado.
    fn chat_de(r: &Path, n: usize) -> Session {
        let s = Session::open_new(r, "m").unwrap();
        let h: Vec<Message> = (0..n).map(|i| if i % 2 == 0 { Message::user(format!("orden {i}")) } else { Message::assistant(format!("respuesta {i}")) }).collect();
        s.save(&h).unwrap();
        s.save_state(&Estado { tools_loaded: vec!["browser".into(), "lsp".into()] }).unwrap();
        s
    }

    /// F8.1: `fork` de un chat de 10 mensajes hasta el 6 deja 7; el padre, intacto; el hijo conserva el estado
    /// y apunta a su padre.
    #[test]
    fn fork_copia_hasta_el_mensaje_y_no_mas() {
        let r = root("fork");
        let padre = chat_de(&r, 10);
        let hijo = Session::fork(&r, padre.id, 6).unwrap();
        assert_ne!(hijo.id, padre.id);
        let (_, h) = Session::open(&r, hijo.id).unwrap();
        assert_eq!(h.len(), 7, "0..=6");
        assert_eq!(h[6].content, "orden 6");
        let (_, hp) = Session::open(&r, padre.id).unwrap();
        assert_eq!(hp.len(), 10, "el padre no se toca");
        assert_eq!(hijo.load_state().tools_loaded, ["browser", "lsp"], "el hijo conserva tools_loaded");
        let l = Session::list(&r).unwrap();
        let info = l.iter().find(|s| s.id == hijo.id).unwrap();
        assert_eq!(info.parent, Some((padre.id, 6)));
        assert!(info.title.starts_with("↳ orden 0"));
        // fuera de rango o en medio de una llamada: no
        assert!(Session::fork(&r, padre.id, 10).is_err());
        let s = Session::open_new(&r, "m").unwrap();
        let call = gotv_engine::ToolCall { id: "c".into(), name: "read_file".into(), arguments: serde_json::json!({}) };
        s.save(&[Message::user("x"), Message::assistant_calls("", std::slice::from_ref(&call)), Message::tool_result(&call, "y"), Message::assistant("z")]).unwrap();
        assert!(Session::fork(&r, s.id, 1).is_err(), "en la llamada");
        assert!(Session::fork(&r, s.id, 2).is_err(), "en su resultado");
        assert_eq!(Session::open(&r, Session::fork(&r, s.id, 3).unwrap().id).unwrap().1.len(), 4);
    }

    /// F8.1: `export` lleva lo que se ve: ni avisos, ni llamadas, ni resultados, ni anexos, ni base64 (deja la
    /// ruta del medio si la tenía).
    #[test]
    fn export_no_lleva_lo_interno_ni_base64() {
        let r = root("export");
        let s = Session::open_new(&r, "modelo-x").unwrap();
        let call = gotv_engine::ToolCall { id: "c0".into(), name: "read_file".into(), arguments: serde_json::json!({"path": "secreto.rs"}) };
        let mut con_anexo = Message::user_with_media("mira esto\n\n[adjunto: a.txt — /x]\n```\nCONTENIDO-ANEXO\n```", vec!["data:image/png;base64,QUFBQUFBQQ==".into()]);
        con_anexo.visible = Some("mira esto".into());
        let mut con_ruta = Message::user("y esta");
        con_ruta.images = vec!["/Users/u/foto.png".into()];
        let h = vec![
            Message::system("SISTEMA-SECRETO"),
            con_anexo,
            Message::assistant_calls("", std::slice::from_ref(&call)),
            Message::tool_result(&call, "RESULTADO-HERRAMIENTA"),
            Message::aviso("AVISO-DEL-PROGRAMA"),
            Message::user(crate::mensajes::adjuntos_pedidos(1)),
            Message::assistant("listo, lo leí"),
            con_ruta,
        ];
        s.save(&h).unwrap();
        for f in [Formato::Markdown, Formato::Json] {
            let t = Session::export(&r, s.id, f).unwrap();
            for no in ["SISTEMA-SECRETO", "RESULTADO-HERRAMIENTA", "AVISO-DEL-PROGRAMA", "CONTENIDO-ANEXO", "QUFBQUFBQQ", "base64", "secreto.rs", "view_image"] {
                assert!(!t.contains(no), "{f:?} no debe llevar «{no}»:\n{t}");
            }
            for si in ["mira esto", "listo, lo leí", "y esta", "/Users/u/foto.png"] {
                assert!(t.contains(si), "{f:?} debe llevar «{si}»:\n{t}");
            }
        }
        let j: serde_json::Value = serde_json::from_str(&Session::export(&r, s.id, Formato::Json).unwrap()).unwrap();
        assert_eq!(j["messages"].as_array().unwrap().len(), 3);
        assert_eq!(j["model"], "modelo-x");
        assert!(Session::export(&r, s.id, Formato::Markdown).unwrap().starts_with("# mira esto"), "el título es la primera orden");
    }

    /// F8.1: un id de OTRO proyecto no se abre aquí; renombrar cambia sólo el título.
    #[test]
    fn open_de_otro_proyecto_falla_y_rename_cambia_el_titulo() {
        let (a, b) = (root("open-a"), root("open-b"));
        for _ in 0..4 {
            chat_de(&a, 2);
        }
        let s = chat_de(&b, 2);
        assert!(Session::open(&b, 4).is_err(), "el 4 es de A");
        assert!(Session::de(&b, 4).is_err());
        let (s2, h) = Session::open(&b, s.id).unwrap();
        assert_eq!(h.len(), 2);
        s2.rename("  Mi   chat\nimportante ").unwrap();
        assert!(s2.rename("   ").is_err());
        let l = Session::list(&b).unwrap();
        assert_eq!((l[0].title.as_str(), l[0].messages), ("Mi chat importante", 2));
        // el título automático: la primera orden ESCRITA, en una línea y a 60
        let largo = "palabra ".repeat(20);
        assert_eq!(titulo_de(&[Message::aviso("no"), Message::user(largo)]).unwrap().chars().count(), 60);
    }

    /// F8.1: la papelera marca y no borra; no cambia la lista de los demás; restaurar la devuelve; «vaciar»
    /// sólo borra lo marcado y vencido.
    #[test]
    fn trash_no_cambia_los_demas_y_vaciar_solo_lo_vencido() {
        let r = root("papelera");
        let (x, y, z) = (chat_de(&r, 2), chat_de(&r, 4), chat_de(&r, 6));
        let antes: Vec<i64> = Session::list(&r).unwrap().iter().map(|s| s.id).collect();
        Session::trash(&r, y.id).unwrap();
        let despues: Vec<i64> = Session::list(&r).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(despues, antes.iter().copied().filter(|i| *i != y.id).collect::<Vec<_>>());
        assert!(Session::open(&r, y.id).is_err(), "de la papelera no se abre");
        assert!(ruta_msj(&carpeta(&r), y.id).is_file(), "marca, no borra");
        assert_eq!(Session::papelera(&r).unwrap().iter().map(|s| s.id).collect::<Vec<_>>(), [y.id]);
        // la que se retoma sola no es la de la papelera
        Session::trash(&r, z.id).unwrap();
        assert_eq!(Session::resume_last(&r, None).unwrap().unwrap().0.id, x.id);
        Session::restore(&r, z.id).unwrap();
        assert_eq!(Session::resume_last(&r, None).unwrap().unwrap().0.id, z.id);
        // vaciar: con 30 días no borra lo de hoy; con 0, sí (y sólo lo marcado)
        let docs = crate::artefactos::chat_de(&r, Some(y.id), "").dir().to_path_buf();
        std::fs::create_dir_all(&docs).unwrap();
        assert_eq!(Session::vaciar(&r, DIAS_PAPELERA).unwrap(), 0);
        assert_eq!(Session::vaciar(&r, 0).unwrap(), 1);
        assert!(!ruta_cab(&carpeta(&r), y.id).exists() && !ruta_msj(&carpeta(&r), y.id).exists() && !docs.exists());
        assert_eq!(Session::list(&r).unwrap().len(), 2);
        // la página
        let (p, total) = Session::page(&r, 1, 1).unwrap();
        assert_eq!((p.len(), total, p[0].id), (1, 2, x.id));
    }

    /// Lo archivado por la compactación: se añade, la exportación lleva la conversación ENTERA (sin la memoria de
    /// trabajo), una rama lo hereda y vaciar la papelera no deja el archivo suelto.
    #[test]
    fn lo_compactado_se_archiva_se_exporta_se_hereda_y_se_borra() {
        let r = root("archivo");
        let s = Session::open_new(&r, "m").unwrap();
        assert!(s.archivo().is_empty(), "nunca compactado: nada archivado");
        s.archivar(&[Message::user("lo de antes"), Message::assistant("respuesta de antes")]).unwrap();
        s.archivar(&[Message::user("lo de en medio")]).unwrap();
        assert_eq!(s.archivo().iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["lo de antes", "respuesta de antes", "lo de en medio"]);
        // como queda tras compactar: el sistema, TU orden original y luego la memoria de trabajo
        s.save(&[
            Message::system("s"),
            Message::user("la orden original"),
            Message::user("(Memoria de trabajo — la conversación se compactó…)\nTAREA: x"),
            Message::user("lo de ahora"),
            Message::assistant("respuesta de ahora"),
        ])
        .unwrap();
        let md = Session::export(&r, s.id, Formato::Markdown).unwrap();
        let (a, b) = (md.find("lo de antes").expect("lo archivado se exporta"), md.find("lo de ahora").unwrap());
        assert!(a < b, "en orden: lo archivado primero");
        assert!(!md.contains("Memoria de trabajo"), "la memoria de trabajo no es conversación");
        // una rama hereda la historia que la precede
        let rama = Session::fork(&r, s.id, 4).unwrap();
        assert_eq!(rama.archivo().len(), 3);
        // vaciar la papelera no deja el .arc suelto
        Session::trash(&r, s.id).unwrap();
        assert_eq!(Session::vaciar(&r, 0).unwrap(), 1);
        assert!(!ruta_arc(&carpeta(&r), s.id).exists());
    }

    /// F8.1 · migración 1: una cabecera de la versión 1 se lee (sin padre ni papelera) y al guardar pasa a la
    /// 2; los MENSAJES siguen en la 1 (un GOTV anterior los sigue leyendo).
    #[test]
    fn una_cabecera_v1_se_lee_y_se_migra() {
        let r = root("v1");
        let dir = carpeta(&r);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = Escritor::nuevo(CAB, 1);
        e.i64(5);
        e.texto("2026-11-01 10:00:00");
        e.texto("m");
        e.texto("de antes");
        e.u32(1);
        e.textos(&["lsp".to_string()]);
        codec::guardar(&ruta_cab(&dir, 5), &e.bytes()).unwrap();
        codec::guardar(&ruta_msj(&dir, 5), &mensajes_bytes(&[Message::user("hola")])).unwrap();
        let l = Session::list(&r).unwrap();
        assert_eq!((l[0].id, l[0].title.as_str(), l[0].parent, l[0].deleted_at), (5, "de antes", None, None));
        let (s, h) = Session::open(&r, 5).unwrap();
        s.save(&h).unwrap();
        assert_eq!(codec::version_de(&std::fs::read(ruta_cab(&dir, 5)).unwrap(), CAB), Some(2));
        assert_eq!(codec::version_de(&std::fs::read(ruta_msj(&dir, 5)).unwrap(), MSJ), Some(1));
        assert_eq!(s.load_state().tools_loaded, ["lsp"]);
    }

    /// A MANO, sobre una COPIA de una base real (nunca la original):
    /// `GOTV_MIGRAR_COPIA=/ruta/copia/gotv.db cargo test -p gotv-core --release -- --ignored --nocapture migrar_una_copia_real`
    #[test]
    #[ignore]
    fn migrar_una_copia_real() {
        use rusqlite::Connection;
        let copia = PathBuf::from(std::env::var("GOTV_MIGRAR_COPIA").expect("GOTV_MIGRAR_COPIA"));
        let r = root("copia-real");
        std::fs::create_dir_all(r.join(".gotv")).unwrap();
        std::fs::copy(&copia, r.join(".gotv/gotv.db")).unwrap();
        let c = Connection::open_with_flags(&copia, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut st = c.prepare("SELECT session_id, COUNT(*) FROM messages GROUP BY session_id ORDER BY session_id DESC").unwrap();
        let antes: Vec<(i64, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(|x| x.ok()).collect();
        let t = std::time::Instant::now();
        let despues: Vec<(i64, i64)> = Session::list(&r).unwrap().iter().map(|s| (s.id, s.messages as i64)).collect();
        println!("{} sesiones · {} mensajes → {} sesiones · {} mensajes en {:.0} ms", antes.len(), antes.iter().map(|x| x.1).sum::<i64>(), despues.len(), despues.iter().map(|x| x.1).sum::<i64>(), t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(antes.into_iter().take(LISTA_MAX).collect::<Vec<_>>(), despues, "cada sesión con los mismos mensajes");
        let (_, h) = Session::resume_last(&r, None).unwrap().unwrap();
        println!("la más reciente: {} mensajes, todos leídos de vuelta", h.len());
    }
}
