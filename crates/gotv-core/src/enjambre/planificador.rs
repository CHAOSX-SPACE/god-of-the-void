//! F9 · el planificador: cuántos subagentes generan A LA VEZ.
//!
//! - **Nube**: cada conversación es una petición aparte → en paralelo, hasta el tope (los créditos y el
//!   límite de peticiones del proveedor son del usuario: el tope los cuida).
//! - **Local con lotes** (el motor dice `lote_max() ≥ 2`): varias conversaciones en un mismo paso sobre los
//!   mismos pesos (F9.0 mide si compensa).
//! - **Local sin lotes**: por TURNOS, uno detrás de otro. Intercalarlos no ayuda: el núcleo tiene UNA caché
//!   de conversación en memoria y cada cambio de subagente la releería entera.
//!
//! El tope se fija con `GOTV_SUBAGENTES` (1–16; por omisión [`TOPE`]).

/// Cuántos subagentes a la vez, como mucho, si nadie dice otra cosa.
pub const TOPE: usize = 4;
/// El techo de `GOTV_SUBAGENTES` (más es pedirle al proveedor una ráfaga que no aguanta).
pub const TOPE_MAXIMO: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// En paralelo (motor de nube): `n` a la vez.
    Paralelo(usize),
    /// En lote (motor local que genera varias conversaciones en un mismo paso): `n` a la vez.
    Lotes(usize),
    /// De a uno.
    Turnos,
}

impl Modo {
    /// Cuántos corren a la vez.
    pub fn a_la_vez(self) -> usize {
        match self {
            Modo::Paralelo(n) | Modo::Lotes(n) => n.max(1),
            Modo::Turnos => 1,
        }
    }

    /// Para el registro del chat.
    pub fn describir(self) -> String {
        match self {
            Modo::Paralelo(n) => crate::tr!("en paralelo ({n} a la vez)", "in parallel ({n} at a time)"),
            Modo::Lotes(n) => crate::tr!("en lote sobre el modelo local ({n} a la vez)", "batched on the local model ({n} at a time)"),
            Modo::Turnos => crate::i18n::t("por turnos (de a uno)", "in turns (one at a time)").to_string(),
        }
    }
}

/// El tope configurado (`GOTV_SUBAGENTES`), entre 1 y [`TOPE_MAXIMO`].
pub fn tope() -> usize {
    tope_de(std::env::var("GOTV_SUBAGENTES").ok().as_deref())
}

/// El tope para un proyecto: `GOTV_SUBAGENTES` si está; si no, `subagentes` de la config (Configuración, o
/// lo que la config del proyecto BAJE), siempre entre 1 y [`TOPE_MAXIMO`].
pub fn tope_en(root: &std::path::Path) -> usize {
    match std::env::var("GOTV_SUBAGENTES") {
        Ok(v) => tope_de(Some(&v)),
        Err(_) => (crate::Config::load(root).subagentes as usize).clamp(1, TOPE_MAXIMO),
    }
}

pub(crate) fn tope_de(valor: Option<&str>) -> usize {
    valor.and_then(|v| v.trim().parse::<usize>().ok()).map(|n| n.clamp(1, TOPE_MAXIMO)).unwrap_or(TOPE)
}

/// Decide el modo para `pedidos` subagentes de una vez.
pub fn decidir(local: bool, lote_max: usize, pedidos: usize, tope: usize) -> Modo {
    let n = pedidos.min(tope.max(1));
    if n < 2 {
        return Modo::Turnos;
    }
    if !local {
        return Modo::Paralelo(n);
    }
    match lote_max.min(n) {
        l if l >= 2 => Modo::Lotes(l),
        _ => Modo::Turnos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_nube_en_paralelo_y_lo_local_segun_su_lote() {
        // nube: en paralelo, con tope
        assert_eq!(decidir(false, 1, 3, 4), Modo::Paralelo(3));
        assert_eq!(decidir(false, 1, 9, 4), Modo::Paralelo(4));
        // local sin lotes (lo de hoy): por turnos, aunque pidan muchos
        assert_eq!(decidir(true, 1, 3, 4), Modo::Turnos);
        // local con lotes (si F9.0 lo confirma y el núcleo lo implementa)
        assert_eq!(decidir(true, 4, 3, 4), Modo::Lotes(3));
        assert_eq!(decidir(true, 2, 3, 4), Modo::Lotes(2));
        // uno solo, o tope 1: de a uno
        assert_eq!(decidir(false, 1, 1, 4), Modo::Turnos);
        assert_eq!(decidir(false, 1, 3, 1), Modo::Turnos);
        assert_eq!(Modo::Paralelo(3).a_la_vez(), 3);
        assert_eq!(Modo::Turnos.a_la_vez(), 1);
    }

    #[test]
    fn el_tope_se_lee_y_se_acota() {
        assert_eq!(tope_de(None), TOPE);
        assert_eq!(tope_de(Some("2")), 2);
        assert_eq!(tope_de(Some("0")), 1);
        assert_eq!(tope_de(Some("99")), TOPE_MAXIMO);
        assert_eq!(tope_de(Some("muchos")), TOPE);
    }
}
