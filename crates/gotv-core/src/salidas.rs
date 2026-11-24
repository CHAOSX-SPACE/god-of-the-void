//! F6 · Salidas largas a archivo: que nada de lo que devuelve una herramienta se pierda al recortarlo.
//!
//! Antes, toda salida de más de `tools::TOOL_MAX_CHARS` (26 000 caracteres) se cortaba por el final y
//! lo cortado desaparecía: un `grep` enorme, una página larga o un informe de pruebas perdían su cola,
//! justo donde suelen estar los errores. Ahora la salida completa se guarda en
//! `.gotv/salidas/<chat>/<n>-<herramienta>.txt` y el modelo ve la cabeza, un aviso con la ruta y la
//! cola; si la herramienta FALLÓ, ve más cola que cabeza (los errores van al final).
//!
//! Las salidas NO se borran al cerrar el chat (la ficha lo pedía así): una conversación se retoma al
//! reabrir la app, y su historial apunta a esos archivos. Se podan por tamaño (200 MB por proyecto) y
//! por edad (7 días), los más viejos primero, y siempre por su ruta exacta, sólo dentro de
//! `.gotv/salidas` (L6). `.gotv` ya queda fuera de los puntos de restauración, del índice y del mapa.
use std::path::{Path, PathBuf};

/// Tope de lo guardado por proyecto.
pub const TOPE_BYTES: u64 = 200 * 1024 * 1024;
/// Edad máxima de una salida guardada.
pub const EDAD_MAX: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// La carpeta de salidas del proyecto.
pub fn carpeta(root: &Path) -> PathBuf {
    root.join(".gotv").join("salidas")
}

/// El nombre de la carpeta de un chat: la parte del dueño tras el último `#`, sólo con caracteres
/// seguros (el dueño es «proyecto#chat-<pid>-<n>»).
fn chat_de(dueno: &str) -> String {
    let c: String = dueno.rsplit('#').next().unwrap_or(dueno).chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let c = c.trim_matches('_').chars().take(48).collect::<String>();
    if c.is_empty() { "directo".into() } else { c }
}

/// El índice de byte en `s` que corresponde al carácter `n` (o el final).
fn byte_del_caracter(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// Si `texto` pasa del tope: lo guarda entero y devuelve cabeza + aviso + cola. Si no, igual.
/// `fallo` cambia el reparto: más cola que cabeza.
pub fn acotar(root: &Path, dueno: &str, herramienta: &str, texto: String, fallo: bool) -> String {
    acotar_con(root, dueno, herramienta, texto, fallo, crate::tools::TOOL_MAX_CHARS)
}

pub(crate) fn acotar_con(root: &Path, dueno: &str, herramienta: &str, texto: String, fallo: bool, tope: usize) -> String {
    let total = texto.chars().count();
    if total <= tope {
        return texto;
    }
    let (p_cabeza, p_cola) = if fallo { (25, 65) } else { (60, 30) };
    let mut fin_cabeza = byte_del_caracter(&texto, tope * p_cabeza / 100);
    let mut ini_cola = byte_del_caracter(&texto, total - tope * p_cola / 100);
    // se corta en un salto de línea cercano (un renglón partido confunde más que uno que falta)
    if let Some(i) = texto[..fin_cabeza].rfind('\n').filter(|i| fin_cabeza - i < 400) {
        fin_cabeza = i + 1;
    }
    if let Some(i) = texto[ini_cola..].find('\n').filter(|i| *i < 400) {
        ini_cola += i + 1;
    }
    let omitidas = texto[fin_cabeza..ini_cola].lines().count();
    let guardado = guardar(root, dueno, herramienta, &texto);
    let aviso = match &guardado {
        Ok(rel) => crate::tr!(
            "\n…[{omitidas} líneas omitidas · completo en {rel} · léelo con read_file (offset/limit) o búscalo con grep]\n",
            "\n…[{omitidas} lines omitted · full output in {rel} · read it with read_file (offset/limit) or search it with grep]\n"
        ),
        Err(e) => crate::tr!(
            "\n…[{omitidas} líneas omitidas · no se pudo guardar la salida completa ({e}): acota el comando o fíltralo]\n",
            "\n…[{omitidas} lines omitted · the full output could not be saved ({e}): narrow or filter the command]\n"
        ),
    };
    format!("{}{aviso}{}", &texto[..fin_cabeza], &texto[ini_cola..])
}

/// Guarda la salida completa; devuelve su ruta relativa al proyecto.
fn guardar(root: &Path, dueno: &str, herramienta: &str, texto: &str) -> std::io::Result<String> {
    let dir = carpeta(root).join(chat_de(dueno));
    std::fs::create_dir_all(&dir)?;
    // el número siguiente al mayor que ya haya (no el recuento: con la poda habría huecos y choques)
    let n = std::fs::read_dir(&dir)?
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().split('-').next().and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
        + 1;
    let herramienta: String = herramienta.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).take(32).collect();
    let ruta = dir.join(format!("{n}-{herramienta}.txt"));
    std::fs::write(&ruta, texto)?;
    podar(root);
    Ok(ruta.strip_prefix(root).unwrap_or(&ruta).display().to_string())
}

/// Poda `.gotv/salidas`: fuera lo de más de `EDAD_MAX` y, si aún pasa de `TOPE_BYTES`, lo más viejo.
/// Sólo archivos de esa carpeta, a dos niveles (`<chat>/<archivo>`), por su ruta exacta.
pub fn podar(root: &Path) {
    podar_con(root, TOPE_BYTES, EDAD_MAX)
}

fn podar_con(root: &Path, tope: u64, edad: std::time::Duration) {
    let base = carpeta(root);
    let Ok(chats) = std::fs::read_dir(&base) else { return };
    let mut archivos: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for chat in chats.flatten() {
        if !chat.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for e in std::fs::read_dir(chat.path()).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_file() {
                archivos.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()));
            }
        }
    }
    archivos.sort_by_key(|a| a.0);
    let ahora = std::time::SystemTime::now();
    let mut total: u64 = archivos.iter().map(|a| a.1).sum();
    for (cuando, peso, ruta) in &archivos {
        let viejo = ahora.duration_since(*cuando).is_ok_and(|d| d > edad);
        if viejo || total > tope {
            if std::fs::remove_file(ruta).is_ok() {
                total = total.saturating_sub(*peso);
            }
        }
    }
    // las carpetas de chat que quedaron vacías, por su ruta exacta
    for chat in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        if chat.file_type().is_ok_and(|t| t.is_dir()) && std::fs::read_dir(chat.path()).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(chat.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lineas(n: usize) -> String {
        (1..=n).map(|i| format!("línea {i} ñ\n")).collect()
    }

    /// Bajo el tope vuelve intacto y no crea nada; encima, deja el archivo completo y devuelve cabeza
    /// + aviso + cola, sin partir un carácter de varios bytes ni un renglón.
    #[test]
    fn lo_largo_se_guarda_entero_y_se_ve_cabeza_aviso_y_cola() {
        let d = crate::CarpetaPrueba::nueva("gotv-salidas");
        let corto = lineas(10);
        assert_eq!(acotar_con(&d, "/p#chat-1-1", "grep", corto.clone(), false, 5_000), corto);
        assert!(!carpeta(&d).exists());
        let largo = lineas(3_000);
        let v = acotar_con(&d, "/p#chat-1-1", "grep", largo.clone(), false, 5_000);
        assert!(v.starts_with("línea 1 ñ\n") && v.ends_with("línea 3000 ñ\n"), "cabeza y cola enteras");
        assert!(v.chars().count() < 5_400, "{}", v.chars().count());
        let ruta = d.join(".gotv/salidas/chat-1-1/1-grep.txt");
        assert_eq!(std::fs::read_to_string(&ruta).unwrap(), largo, "el archivo es la salida COMPLETA");
        assert!(v.contains(".gotv/salidas/chat-1-1/1-grep.txt") && v.contains("líneas omitidas"), "{}", &v[v.find('…').unwrap()..][..200]);
        // ningún renglón partido: cada línea visible es una línea entera del original
        assert!(v.lines().filter(|l| l.starts_with("línea")).all(|l| l.ends_with(" ñ")));
        // la segunda va con el número siguiente
        acotar_con(&d, "/p#chat-1-1", "web fetch", largo.clone(), false, 5_000);
        assert!(d.join(".gotv/salidas/chat-1-1/2-web_fetch.txt").is_file());
    }

    /// Si falló, se ve más cola que cabeza (el error suele estar al final).
    #[test]
    fn con_fallo_se_ve_mas_cola() {
        let d = crate::CarpetaPrueba::nueva("gotv-salidas-fallo");
        let mut t = lineas(3_000);
        t.push_str("error[E0308]: mismatched types en la línea 2990\n");
        let bien = acotar_con(&d, "x#c", "bash", t.clone(), false, 5_000);
        let mal = acotar_con(&d, "x#c", "bash", t, true, 5_000);
        let cola = |v: &str| v.len() - v.find('…').unwrap();
        assert!(cola(&mal) > cola(&bien) * 2, "{} vs {}", cola(&mal), cola(&bien));
        assert!(mal.contains("error[E0308]"));
    }

    /// La poda borra sólo dentro de `.gotv/salidas`, primero lo viejo, hasta quedar bajo el tope.
    #[test]
    fn la_poda_respeta_el_tope_y_no_sale_de_su_carpeta() {
        let d = crate::CarpetaPrueba::nueva("gotv-salidas-poda");
        std::fs::create_dir_all(d.join(".gotv/salidas/c1")).unwrap();
        std::fs::write(d.join("del-proyecto.txt"), "x".repeat(5_000)).unwrap();
        std::fs::write(d.join(".gotv/fuera.txt"), "x".repeat(5_000)).unwrap();
        for i in 1..=5 {
            std::fs::write(d.join(format!(".gotv/salidas/c1/{i}-grep.txt")), "x".repeat(1_000)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        podar_con(&d, 2_500, EDAD_MAX);
        let quedan: Vec<bool> = (1..=5).map(|i| d.join(format!(".gotv/salidas/c1/{i}-grep.txt")).exists()).collect();
        assert_eq!(quedan, [false, false, false, true, true], "se van los más viejos hasta caber");
        assert!(d.join("del-proyecto.txt").exists() && d.join(".gotv/fuera.txt").exists(), "nada fuera de su carpeta");
        // por edad: con edad 0, todo fuera, y la carpeta del chat vacía también
        podar_con(&d, TOPE_BYTES, std::time::Duration::ZERO);
        assert!(!d.join(".gotv/salidas/c1").exists());
        assert!(d.join(".gotv/salidas").is_dir() && d.join(".gotv/fuera.txt").exists());
    }

    #[test]
    fn el_chat_sale_del_dueno_con_caracteres_seguros() {
        assert_eq!(chat_de("/Users/x/proyecto#chat-123-4"), "chat-123-4");
        assert_eq!(chat_de("/x#../../etc"), "etc");
        assert_eq!(chat_de("###"), "directo");
    }
}
