//! El TOPE de la memoria rápida de un proyecto (F3 del plano de la memoria rápida): por omisión 3 GiB y
//! siempre un 10 % libre para el movimiento de entrar lo nuevo y sacar lo viejo (pedido del Portador).
//!
//! Sólo archivos (sin MLX): lo usan el núcleo (al guardar) y la app (al cambiar el tope o mostrar lo
//! ocupado). La carpeta de un proyecto es `<base>/<proyecto>/` con:
//!
//! ```text
//! tope                  ← bytes del tope (texto); si falta, 3 GiB
//! s<sesión>/<clave>/    ← una ENTRADA: la memoria de una conversación con un modelo (F2)
//! s<sesión>/<clave>.*   ← entrada del formato anterior (.safetensors + .ids + .imgs)
//! ```
//!
//! Se desaloja la entrada usada hace más tiempo (la fecha de su `indice.gotv` o `.ids`, que se toca al
//! leerla), NUNCA tokens sueltos: una conversación se conserva entera o como el prefijo que cupo.
//! Sólo se tocan carpetas `s<número>` (falla #2571: una poda que no lo comprobaba borró carpetas del
//! directorio temporal de macOS).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 3 GiB.
pub const TOPE_POR_OMISION: u64 = 3 << 30;
/// Topes que ofrece la app (GiB).
pub const TOPES_GB: [u64; 4] = [3, 6, 12, 24];

/// ¿Es una carpeta de sesión (`s<número>`)?
pub fn es_sesion(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix('s')).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// El tope de la carpeta de un proyecto, en bytes. `GOTV_CONVERSACIONES_GB` lo fuerza (pruebas).
pub fn tope(proyecto: &Path) -> u64 {
    if let Some(gb) = std::env::var("GOTV_CONVERSACIONES_GB").ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|g| *g > 0.0) {
        return (gb * (1u64 << 30) as f64) as u64;
    }
    std::fs::read_to_string(proyecto.join("tope")).ok().and_then(|t| t.trim().parse().ok()).filter(|&b: &u64| b > 0).unwrap_or(TOPE_POR_OMISION)
}

/// Fija el tope (bytes) de la carpeta de un proyecto.
pub fn fijar_tope(proyecto: &Path, bytes: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(proyecto)?;
    std::fs::write(proyecto.join("tope"), bytes.to_string())
}

/// Lo que puede ocupar de verdad: el tope menos el 10 % que siempre queda libre.
pub fn utilizable(tope: u64) -> u64 {
    tope / 10 * 9
}

fn bytes_de(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| bytes_de(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Una entrada de la memoria: sus rutas (carpeta, o los archivos del formato anterior), bytes y edad.
#[derive(Debug, Clone)]
pub struct Entrada {
    /// la carpeta de la entrada (formato nuevo) o la ruta base sin extensión (formato anterior)
    pub base: PathBuf,
    pub rutas: Vec<PathBuf>,
    pub bytes: u64,
    pub usada: SystemTime,
}

/// Las entradas de la carpeta de un proyecto (sólo dentro de `s<número>`).
pub fn entradas(proyecto: &Path) -> Vec<Entrada> {
    let mut v = Vec::new();
    for s in std::fs::read_dir(proyecto).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir() && es_sesion(p)) {
        let mut viejas: std::collections::BTreeMap<PathBuf, Vec<PathBuf>> = Default::default();
        for p in std::fs::read_dir(&s).into_iter().flatten().flatten().map(|e| e.path()) {
            if p.is_dir() {
                let marca = p.join("indice.gotv");
                let usada = std::fs::metadata(&marca).and_then(|m| m.modified()).or_else(|_| std::fs::metadata(&p).and_then(|m| m.modified())).unwrap_or(SystemTime::UNIX_EPOCH);
                v.push(Entrada { base: p.clone(), bytes: bytes_de(&p), rutas: vec![p], usada });
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("safetensors" | "ids" | "imgs")) {
                viejas.entry(p.with_extension("")).or_default().push(p);
            }
        }
        for (base, rutas) in viejas {
            let usada = std::fs::metadata(base.with_extension("ids")).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let bytes = rutas.iter().map(|r| bytes_de(r)).sum();
            v.push(Entrada { base, rutas, bytes, usada });
        }
    }
    v
}

/// Bytes que ocupa la memoria de un proyecto.
pub fn ocupado(proyecto: &Path) -> u64 {
    entradas(proyecto).iter().map(|e| e.bytes).sum()
}

/// Marca una entrada como recién usada (al leerla): así el desalojo es por USO, no por escritura.
pub fn tocar(entrada: &Path) {
    let p = entrada.join("indice.gotv");
    if let Ok(f) = std::fs::File::options().write(true).open(&p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Hace sitio para `necesita` bytes más en la entrada `protegida` (que ya ocupa `ya`): desaloja las
/// entradas usadas hace más tiempo, enteras, hasta que todo quepa en el 90 % del tope. Devuelve si
/// cabe. Nunca toca `protegida`. Borra también las carpetas de sesión que queden vacías.
pub fn hacer_sitio(proyecto: &Path, protegida: &Path, ya: u64, necesita: u64, tope: u64) -> bool {
    let limite = utilizable(tope);
    if ya + necesita > limite {
        return false; // ni vaciando lo demás cabe
    }
    let mut otras: Vec<Entrada> = entradas(proyecto).into_iter().filter(|e| e.base != protegida).collect();
    otras.sort_by_key(|e| e.usada);
    let mut usado: u64 = otras.iter().map(|e| e.bytes).sum::<u64>() + ya;
    for e in otras {
        if usado + necesita <= limite {
            break;
        }
        for r in &e.rutas {
            let _ = if r.is_dir() { std::fs::remove_dir_all(r) } else { std::fs::remove_file(r) };
        }
        usado = usado.saturating_sub(e.bytes);
    }
    for s in std::fs::read_dir(proyecto).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir() && es_sesion(p)) {
        // `remove_dir` sólo borra carpetas vacías
        let _ = std::fs::remove_dir(&s);
    }
    usado + necesita <= limite
}

/// Bytes libres en el disco donde vive `p` (o en su carpeta existente más cercana).
pub fn libre_en_disco(p: &Path) -> Option<u64> {
    let mut q = p.to_path_buf();
    while !q.exists() {
        q = q.parent()?.to_path_buf();
    }
    let c = std::ffi::CString::new(q.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` es una ruta terminada en cero y `s` una estructura válida que statvfs rellena
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (guardia, carpeta del proyecto): la guardia borra toda la raíz al soltarse.
    fn raiz(tag: &str) -> (crate::prueba::CarpetaPrueba, PathBuf) {
        let g = crate::prueba::CarpetaPrueba::nueva(&format!("gotv-tope-{tag}"));
        let d = g.join("proyecto");
        std::fs::create_dir_all(&d).unwrap();
        (g, d)
    }

    /// Una entrada falsa de `mb` MB usada hace `hace` segundos.
    fn entrada(proyecto: &Path, sesion: &str, clave: &str, mb: u64, hace: u64) -> PathBuf {
        let d = proyecto.join(sesion).join(clave);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("t-0000000.gkv"), vec![0u8; (mb << 20) as usize]).unwrap();
        std::fs::write(d.join("indice.gotv"), b"{}").unwrap();
        let f = std::fs::File::options().write(true).open(d.join("indice.gotv")).unwrap();
        f.set_modified(SystemTime::now() - std::time::Duration::from_secs(hace)).unwrap();
        d
    }

    #[test]
    fn desaloja_lo_usado_hace_mas_tiempo_y_deja_el_10_por_ciento() {
        let (_raiz, p) = raiz("lru");
        let tope = 100 << 20;
        let vieja = entrada(&p, "s1", "a", 40, 300);
        let media = entrada(&p, "s2", "a", 40, 200);
        let actual = entrada(&p, "s3", "a", 5, 10);
        // 85 MB ocupados; añadir 10 MB a la actual pasa del 90 % → sale la más vieja
        assert!(hacer_sitio(&p, &actual, 5 << 20, 10 << 20, tope));
        assert!(!vieja.exists() && media.exists() && actual.exists());
        assert!(!p.join("s1").exists(), "la sesión vacía se borra");
        assert!(ocupado(&p) + (10 << 20) <= utilizable(tope));
        // leer la «media» la vuelve la más reciente: ahora sale otra
        let otra = entrada(&p, "s4", "a", 30, 100);
        tocar(&media);
        assert!(hacer_sitio(&p, &actual, 5 << 20, 30 << 20, tope));
        assert!(media.exists() && !otra.exists(), "se desaloja por USO, no por escritura");
    }

    #[test]
    fn lo_que_no_cabe_ni_vaciando_no_se_escribe_y_nada_se_borra() {
        let (_raiz, p) = raiz("grande");
        let otra = entrada(&p, "s1", "a", 20, 100);
        let actual = entrada(&p, "s2", "a", 10, 10);
        assert!(!hacer_sitio(&p, &actual, 10 << 20, 85 << 20, 100 << 20), "10 + 85 > 90");
        assert!(otra.exists(), "si no cabe ni vaciando, no se desaloja nada en vano");
    }

    #[test]
    fn solo_toca_sesiones_y_cuenta_el_formato_anterior() {
        let (_raiz, p) = raiz("forma");
        std::fs::create_dir_all(p.join("notas")).unwrap();
        std::fs::write(p.join("notas").join("x"), vec![0u8; 50 << 20]).unwrap();
        let s = p.join("s1");
        std::fs::create_dir_all(&s).unwrap();
        for e in ["safetensors", "ids", "imgs"] {
            std::fs::write(s.join(format!("7ac1.{e}")), vec![0u8; 1 << 20]).unwrap();
        }
        let actual = entrada(&p, "s2", "a", 5, 1);
        let es = entradas(&p);
        assert_eq!(es.len(), 2, "la del formato anterior cuenta como UNA entrada; «notas» no");
        assert!(hacer_sitio(&p, &actual, 5 << 20, 4 << 20, 10 << 20));
        assert!(!s.join("7ac1.safetensors").exists() && !s.join("7ac1.ids").exists(), "sale entera");
        assert!(p.join("notas").join("x").exists(), "lo que no es sesión jamás se toca");
    }

    #[test]
    fn el_tope_se_lee_se_fija_y_tiene_omision() {
        let (_raiz, p) = raiz("valor");
        assert_eq!(tope(&p), TOPE_POR_OMISION);
        fijar_tope(&p, 6 << 30).unwrap();
        assert_eq!(tope(&p), 6 << 30);
        assert_eq!(utilizable(100), 90);
        assert!(libre_en_disco(&p.join("no/existe/aun")).is_some_and(|b| b > 0));
    }
}
