//! Bilingüe EN/ES (P8). Cada texto visible nace como PAR — `tr!("es", "en")` —
//! para que ninguna zona quede con el idioma cableado (lección de la falla #53:
//! un ayudante de idioma definido tarde deja una zona muerta arriba).
//!
//! El idioma es global al proceso: lo fija el arranque (config `lang`, `GOTV_LANG`,
//! o el idioma del sistema) y no cambia durante una orden. En pruebas, un
//! sobreescrito por hilo evita carreras entre pruebas en paralelo.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static OVERRIDE: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) };
}

pub fn set(l: Lang) {
    LANG.store(if l == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if let Some(l) = OVERRIDE.with(|o| o.get()) {
        return l;
    }
    if LANG.load(Ordering::Relaxed) == 1 { Lang::En } else { Lang::Es }
}

pub fn is_en() -> bool {
    get() == Lang::En
}

/// Idioma de ESTE hilo (pruebas de cualquier crate: no tocan el idioma global de las demás).
pub fn set_thread(l: Option<Lang>) {
    OVERRIDE.with(|o| o.set(l));
}

/// Elige texto fijo según el idioma.
pub fn t(es: &'static str, en: &'static str) -> &'static str {
    if is_en() { en } else { es }
}

/// `es` | `en` | `auto`/vacío. `auto` mira `GOTV_LANG`, luego el idioma del
/// sistema (macOS `AppleLanguages`), luego `LANG`. Sin pista → español.
pub fn resolve(pref: &str) -> Lang {
    let parse = |s: &str| -> Option<Lang> {
        let s = s.trim().trim_matches(|c| c == '"' || c == '(' || c == ')').to_lowercase();
        if s.starts_with("en") {
            Some(Lang::En)
        } else if s.starts_with("es") {
            Some(Lang::Es)
        } else {
            None
        }
    };
    if let Some(l) = parse(pref) {
        return l;
    }
    if let Some(l) = std::env::var("GOTV_LANG").ok().as_deref().and_then(parse) {
        return l;
    }
    if let Ok(o) = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output() {
        let txt = String::from_utf8_lossy(&o.stdout);
        if let Some(first) = txt.split([',', '\n']).map(str::trim).find(|s| s.len() > 1 && s != &"(") {
            if let Some(l) = parse(first) {
                return l;
            }
            return Lang::En; // otro idioma del sistema: inglés como lengua franca
        }
    }
    std::env::var("LANG").ok().as_deref().and_then(parse).unwrap_or(Lang::Es)
}

/// `tr!("texto {x}", "text {x}")` → `String` en el idioma activo (con `format!`).
#[macro_export]
macro_rules! tr {
    ($es:literal, $en:literal $(,)?) => {
        if $crate::i18n::is_en() { format!($en) } else { format!($es) }
    };
    ($es:literal, $en:literal, $($arg:tt)+) => {
        if $crate::i18n::is_en() { format!($en, $($arg)+) } else { format!($es, $($arg)+) }
    };
}

/// `anyhow_tr!("es {x}", "en {x}")` = `anyhow::anyhow!(tr!(…))`: un error en el idioma activo.
#[macro_export]
macro_rules! anyhow_tr {
    ($($t:tt)+) => {
        ::anyhow::anyhow!($crate::tr!($($t)+))
    };
}

/// `bail_tr!("es {x}", "en {x}")` = `anyhow::bail!(tr!(…))`.
#[macro_export]
macro_rules! bail_tr {
    ($($t:tt)+) => {
        return ::core::result::Result::Err($crate::anyhow_tr!($($t)+).into())
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errores_en_el_idioma_activo() {
        fn falla(n: u32) -> anyhow::Result<()> {
            bail_tr!("{n} archivos de más", "{n} files too many");
        }
        set_thread(Some(Lang::En));
        assert_eq!(falla(2).unwrap_err().to_string(), "2 files too many");
        assert_eq!(anyhow_tr!("sin {}", "no {}", "datos").to_string(), "no datos");
        set_thread(Some(Lang::Es));
        assert_eq!(falla(2).unwrap_err().to_string(), "2 archivos de más");
        set_thread(None);
    }

    #[test]
    fn pares_y_formato() {
        set_thread(Some(Lang::En));
        let n = 3;
        assert_eq!(tr!("{n} archivos", "{n} files"), "3 files");
        assert_eq!(t("hola", "hello"), "hello");
        set_thread(Some(Lang::Es));
        assert_eq!(tr!("{n} archivos", "{n} files"), "3 archivos");
        assert_eq!(tr!("{} de {}", "{} of {}", 1, 2), "1 de 2");
        set_thread(None);
    }

    #[test]
    fn resuelve_preferencias() {
        assert_eq!(resolve("en"), Lang::En);
        assert_eq!(resolve("ES"), Lang::Es);
        assert_eq!(resolve("en-US"), Lang::En);
    }
}
