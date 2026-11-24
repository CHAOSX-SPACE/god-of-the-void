//! F8.3 · Menciones con @: el autocompletado de la caja de escribir y lo que una mención trae al mensaje.
//!
//! Se ofrecen archivos y carpetas del proyecto (con el crate `ignore`: respeta `.gitignore`, `.ignore` y
//! los excluidos globales; nunca `.git` ni `.gotv`), los chats del proyecto, sus reglas (F12) y los
//! recursos de los servidores MCP activos (F13.6).
//!
//! Lo que se inserta en el texto es una FICHA legible (`@src/a.rs`, `@chat#12`, `@mcp:servidor:uri`):
//! - un archivo, carpeta o regla queda como su ruta (el agente la lee con sus herramientas);
//! - un chat viaja como ANEXO al mensaje (su exportación en Markdown, recortada): el agente no puede leer
//!   otro chat por su cuenta;
//! - un recurso MCP también viaja como anexo (lo lee quien tiene la conexión: el worker).
//!
//! Los anexos empiezan por `[adjunto: ` (los que `mensajes::visible` ya sabe quitar), así el chat muestra
//! sólo lo que escribió el Portador.

use crate::session::{Formato, Session};
use serde::Serialize;
use std::path::Path;

/// Lo más que se ofrece en la lista.
pub const MAX: usize = 40;
/// Tope de lo que viaja de un chat o recurso mencionado.
pub const ANEXO_MAX_CHARS: usize = 12_000;
/// Archivos que se miran como mucho al buscar (un proyecto enorme no congela la caja).
const RECORRIDO_MAX: usize = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Clase {
    Archivo,
    Carpeta,
    Chat,
    Regla,
    Recurso,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Mencion {
    pub clase: Clase,
    /// lo que se ve en la lista
    pub etiqueta: String,
    /// lo que se inserta en el texto (sin la @)
    pub ficha: String,
    /// una pista al lado (carpeta, fecha, servidor…)
    pub detalle: String,
}

/// Qué tan bien casa `objetivo` con la consulta (más alto = mejor; `None` = no casa). La consulta va en
/// minúsculas. Prefijo del nombre > nombre > ruta > letras en orden.
fn puntaje(objetivo: &str, consulta: &str) -> Option<u32> {
    if consulta.is_empty() {
        return Some(1);
    }
    let o = objetivo.to_lowercase();
    let nombre = o.trim_end_matches('/').rsplit('/').next().unwrap_or(&o);
    if nombre.starts_with(consulta) {
        return Some(1000 - nombre.len().min(900) as u32);
    }
    if nombre.contains(consulta) {
        return Some(700 - nombre.len().min(600) as u32);
    }
    if o.contains(consulta) {
        return Some(400 - o.len().min(300) as u32);
    }
    // las letras de la consulta, en orden, dentro de la ruta («srcmain» → src/main.rs)
    let mut it = o.chars();
    consulta.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|x| x == c)).then(|| 50 - o.len().min(49) as u32)
}

/// Archivos y carpetas del proyecto que casan con la consulta, respetando lo ignorado.
pub fn archivos(root: &Path, consulta: &str, max: usize) -> Vec<Mencion> {
    let q = consulta.trim().to_lowercase();
    let mut v: Vec<(u32, Mencion)> = Vec::new();
    let paseo = ignore::WalkBuilder::new(root)
        .hidden(false) // `.github/` y compañía se pueden mencionar…
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false) // …y `.gitignore` vale aunque no haya repositorio
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | ".gotv" | ".DS_Store")))
        .build();
    for e in paseo.flatten().take(RECORRIDO_MAX) {
        let Ok(rel) = e.path().strip_prefix(root) else { continue };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let rel = rel.to_string_lossy().replace('\\', "/");
        let carpeta = e.file_type().is_some_and(|t| t.is_dir());
        let ficha = if carpeta { format!("{rel}/") } else { rel.clone() };
        if let Some(p) = puntaje(&ficha, &q) {
            let detalle = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            let etiqueta = ficha.trim_end_matches('/').rsplit('/').next().unwrap_or(&ficha).to_string() + if carpeta { "/" } else { "" };
            v.push((p, Mencion { clase: if carpeta { Clase::Carpeta } else { Clase::Archivo }, etiqueta, ficha, detalle }));
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.ficha.len().cmp(&b.1.ficha.len())).then_with(|| a.1.ficha.cmp(&b.1.ficha)));
    v.into_iter().take(max).map(|x| x.1).collect()
}

/// Los chats del proyecto que casan (por título).
pub fn chats(root: &Path, consulta: &str, max: usize) -> Vec<Mencion> {
    let q = consulta.trim().to_lowercase();
    Session::list(root)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| q.is_empty() || s.title.to_lowercase().contains(&q) || format!("chat#{}", s.id).contains(&q))
        .take(max)
        .map(|s| Mencion { clase: Clase::Chat, etiqueta: if s.title.is_empty() { format!("#{}", s.id) } else { s.title.clone() }, ficha: format!("chat#{}", s.id), detalle: s.created })
        .collect()
}

/// Las reglas del proyecto (F12: `.gotv/reglas`, `.claude/rules`) que casan.
pub fn reglas(root: &Path, consulta: &str, max: usize) -> Vec<Mencion> {
    let q = consulta.trim().to_lowercase();
    let mut rs: Vec<String> = crate::formas::reglas::cabeceras(root).into_iter().map(|r| r.archivo).collect();
    rs.extend(crate::formas::reglas::siempre(root).iter().map(|p| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")));
    rs.sort();
    rs.dedup();
    rs.into_iter()
        .filter(|r| puntaje(r, &q).is_some())
        .take(max)
        .map(|r| Mencion { clase: Clase::Regla, etiqueta: r.rsplit('/').next().unwrap_or(&r).trim_end_matches(".md").to_string(), detalle: r.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default(), ficha: r })
        .collect()
}

/// Los recursos MCP (servidor, uri, nombre, descripción) que casan.
pub fn recursos(lista: &[(String, String, String, String)], consulta: &str, max: usize) -> Vec<Mencion> {
    let q = consulta.trim().to_lowercase();
    lista
        .iter()
        .filter(|(s, u, n, _)| q.is_empty() || [s, u, n].iter().any(|x| x.to_lowercase().contains(&q)))
        .take(max)
        .map(|(s, u, n, d)| Mencion { clase: Clase::Recurso, etiqueta: if n.is_empty() { u.clone() } else { n.clone() }, ficha: format!("mcp:{s}:{u}"), detalle: if d.is_empty() { s.clone() } else { format!("{s} · {d}") } })
        .collect()
}

/// Todo lo que casa con la consulta, por clases: reglas, chats, recursos y archivos (los archivos llenan lo
/// que quede hasta [`MAX`]).
pub fn buscar(root: &Path, consulta: &str, mcp: &[(String, String, String, String)]) -> Vec<Mencion> {
    let mut v = reglas(root, consulta, 5);
    v.extend(chats(root, consulta, 5));
    v.extend(recursos(mcp, consulta, 8));
    let resto = MAX.saturating_sub(v.len());
    v.extend(archivos(root, consulta, resto));
    v
}

/// Todos los recursos de los servidores MCP activos: (servidor, uri, nombre, descripción).
pub async fn recursos_de(mcp: &mut crate::mcp::Mcp) -> Vec<(String, String, String, String)> {
    let mut out = Vec::new();
    for s in mcp.nombres() {
        if let Ok(rs) = mcp.recursos(&s).await {
            out.extend(rs.into_iter().map(|(u, n, d)| (s.clone(), u, n, d)));
        }
    }
    out
}

/// Las fichas `@chat#N` y `@mcp:servidor:uri` que hay en un texto (sin repetir, en orden).
pub fn fichas(texto: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for t in texto.split_whitespace() {
        let Some(f) = t.strip_prefix('@') else { continue };
        let f = f.trim_end_matches(|c: char| matches!(c, ',' | '.' | ';' | ')' | ']' | '»' | '"'));
        if (f.starts_with("chat#") && f[5..].parse::<i64>().is_ok()) || (f.starts_with("mcp:") && f[4..].contains(':')) {
            if !v.iter().any(|x| x == f) {
                v.push(f.to_string());
            }
        }
    }
    v
}

fn recorta(t: &str) -> String {
    if t.chars().count() <= ANEXO_MAX_CHARS {
        return t.to_string();
    }
    let corto: String = t.chars().take(ANEXO_MAX_CHARS).collect();
    format!("{corto}\n{}", crate::tr!("… (recortado a {ANEXO_MAX_CHARS} caracteres)", "… (cut to {ANEXO_MAX_CHARS} characters)"))
}

/// Un anexo para el mensaje (empieza por `[adjunto: `, que el chat sabe no mostrar).
pub fn anexo(nombre: &str, origen: &str, texto: &str) -> String {
    crate::tr!("[adjunto: {nombre} — {origen}]\n```\n{}\n```", "[attachment: {nombre} — {origen}]\n```\n{}\n```", recorta(texto))
}

/// Los anexos de los chats mencionados (`@chat#N`): su exportación en Markdown. Uno que no existe se dice.
pub fn anexos_de_chats(root: &Path, texto: &str) -> Vec<String> {
    fichas(texto)
        .into_iter()
        .filter_map(|f| f.strip_prefix("chat#").and_then(|n| n.parse::<i64>().ok()))
        .map(|id| match Session::export(root, id, Formato::Markdown) {
            Ok(md) => anexo(&format!("chat#{id}"), crate::i18n::t("otro chat de este proyecto", "another chat of this project"), &md),
            Err(e) => anexo(&format!("chat#{id}"), crate::i18n::t("no se pudo leer", "could not be read"), &format!("{e:#}")),
        })
        .collect()
}

/// El texto con sus anexos detrás (tras una línea en blanco cada uno).
pub fn con_anexos(texto: &str, anexos: &[String]) -> String {
    let mut t = texto.to_string();
    for a in anexos {
        t.push_str("\n\n");
        t.push_str(a);
    }
    t
}

/// F8.3: citar un trozo seleccionado (de una respuesta, un archivo o un diff): en bloque de cita, con su
/// origen, listo para la caja de escribir.
pub fn cita(origen: &str, texto: &str) -> String {
    let cuerpo: Vec<String> = texto.trim_end().lines().take(200).map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") }).collect();
    let origen = origen.trim();
    if origen.is_empty() {
        format!("{}\n\n", cuerpo.join("\n"))
    } else {
        format!("{}\n{}\n\n", crate::tr!("De {origen}:", "From {origen}:"), cuerpo.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proyecto(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-menc-{tag}"));
        for (r, t) in [
            ("src/main.rs", "fn main(){}"),
            ("src/lib.rs", ""),
            ("src/util/cadena.rs", ""),
            ("dist/bundle.js", "x"),
            ("secretos/clave.txt", "x"),
            ("notas.log", "x"),
            (".github/workflows/ci.yml", ""),
            (".gotv/reglas/sql.md", "---\nrutas: [\"**/*.sql\"]\n---\nnumeradas"),
            (".gitignore", "dist/\nsecretos/\n*.log\n"),
        ] {
            let p = d.join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        d
    }

    /// El autocompletado NO ofrece lo ignorado (`.gitignore` aunque no haya repositorio), ni `.git`/`.gotv`.
    #[test]
    fn el_autocompletado_no_ofrece_lo_ignorado() {
        let r = proyecto("ign");
        let todo: Vec<String> = archivos(&r, "", 500).into_iter().map(|m| m.ficha).collect();
        for no in ["dist/bundle.js", "dist/", "secretos/clave.txt", "notas.log", ".gotv/reglas/sql.md"] {
            assert!(!todo.contains(&no.to_string()), "{no} es ignorado: {todo:?}");
        }
        for si in ["src/main.rs", "src/", "src/util/cadena.rs", ".github/workflows/ci.yml", ".gitignore"] {
            assert!(todo.contains(&si.to_string()), "{si} debe estar: {todo:?}");
        }
        // el orden: el prefijo del nombre primero
        let m = archivos(&r, "ma", 5);
        assert_eq!(m[0].ficha, "src/main.rs");
        assert_eq!((m[0].etiqueta.as_str(), m[0].detalle.as_str(), m[0].clase), ("main.rs", "src", Clase::Archivo));
        assert_eq!(archivos(&r, "util", 5)[0].clase, Clase::Carpeta);
        // letras en orden
        assert!(archivos(&r, "scad", 5).iter().any(|m| m.ficha == "src/util/cadena.rs"));
        assert!(archivos(&r, "zzzz", 5).is_empty());
    }

    #[test]
    fn reglas_chats_y_recursos() {
        let r = proyecto("otros");
        assert_eq!(reglas(&r, "sq", 5)[0].ficha, ".gotv/reglas/sql.md");
        let s = Session::open_new(&r, "m").unwrap();
        s.save(&[gotv_engine::Message::user("arregla el login"), gotv_engine::Message::assistant("hecho")]).unwrap();
        let c = chats(&r, "login", 5);
        assert_eq!((c[0].ficha.as_str(), c[0].etiqueta.as_str()), (format!("chat#{}", s.id).as_str(), "arregla el login"));
        let lista = vec![("docs".to_string(), "mem://nota".to_string(), "nota".to_string(), "una nota".to_string())];
        assert_eq!(recursos(&lista, "not", 5)[0].ficha, "mcp:docs:mem://nota");
        let todo = buscar(&r, "", &lista);
        assert!(todo.len() <= MAX && todo.iter().any(|m| m.clase == Clase::Chat) && todo.iter().any(|m| m.clase == Clase::Recurso));
    }

    /// Un chat mencionado viaja como anexo (su Markdown) y el chat muestra sólo lo escrito.
    #[test]
    fn un_chat_mencionado_viaja_como_anexo() {
        let r = proyecto("anexo");
        let s = Session::open_new(&r, "m").unwrap();
        s.save(&[gotv_engine::Message::user("diseña la tabla de pedidos"), gotv_engine::Message::assistant("La tabla lleva id y total.")]).unwrap();
        let texto = format!("sigue con lo de @chat#{}, y mira @src/main.rs. Y @chat#{}.", s.id, s.id);
        assert_eq!(fichas(&texto), [format!("chat#{}", s.id)]);
        let a = anexos_de_chats(&r, &texto);
        assert_eq!(a.len(), 1);
        assert!(a[0].contains("La tabla lleva id y total."));
        let msg = gotv_engine::Message::user(con_anexos(&texto, &a));
        assert_eq!(crate::mensajes::visible(&msg), texto, "el chat muestra sólo lo escrito");
        assert!(anexos_de_chats(&r, "@chat#999")[0].contains("chat#999"));
        assert_eq!(fichas("@mcp:docs:mem://nota ok"), ["mcp:docs:mem://nota"]);
    }

    #[test]
    fn citar_un_trozo() {
        let c = cita("src/a.rs L3-4", "let x = 1;\n\nlet y = 2;\n");
        assert_eq!(c, crate::tr!("De src/a.rs L3-4:\n> let x = 1;\n>\n> let y = 2;\n\n", "From src/a.rs L3-4:\n> let x = 1;\n>\n> let y = 2;\n\n"));
        assert_eq!(cita("", "hola"), "> hola\n\n");
    }
}
