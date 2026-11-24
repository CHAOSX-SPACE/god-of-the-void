//! O4 · El TEMA de un documento (Word y PDF): los colores y las tipografías con los que GOTV viste un
//! informe, con el mismo criterio que sus páginas (`gotv-diseno`) y sus presentaciones (`ppt`).
//!
//! `tema` en la ficha admite:
//! - `{"marca": "azul" | "#1e40af", "croma"?: "sobrio"|"medio"|"vivo", "fuentes"?: "editorial"…}`: el tema
//!   de `gotv-diseno`, con sus garantías de contraste (texto ≥ 12:1, acento de texto ≥ 4,6:1…);
//! - `{"paleta": "midnight"…}`: una de las paletas de las presentaciones, para que el informe y el deck
//!   de un mismo trabajo vayan a juego;
//! - un texto solo (`"oceano"`, `"verde"`, `"#7c3aed"`): paleta si existe con ese nombre; si no, marca;
//! - `primario`, `fuente_titulo` y `fuente_cuerpo` mandan sobre lo calculado.
//!
//! Tipografías: un .docx NO lleva sus fuentes dentro, así que el par elegido se traduce a fuentes que
//! están en cualquier equipo (serif → Georgia, sin serifa → Arial, mono → Courier New). El PDF sí puede
//! llevar las de `gotv-diseno` incrustadas.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct TemaDoc {
    /// el color de marca para rellenos (cabecera de tabla, filete del título); hex de 6 sin `#`
    pub primario: String,
    /// el texto que se lee SOBRE `primario`
    pub sobre_primario: String,
    /// el color de los títulos: la marca si se lee sobre blanco; si no, su versión oscura
    pub titulo: String,
    pub texto: String,
    pub tenue: String,
    /// el fondo de las filas alternas
    pub suave: String,
    pub borde: String,
    pub fuente_titulo: String,
    pub fuente_cuerpo: String,
    /// el par tipográfico de `gotv-diseno` (para el PDF, que sí incrusta fuentes); vacío con una paleta
    pub par: String,
}

fn limpio(h: &str) -> String {
    h.trim().trim_start_matches('#').to_uppercase()
}

/// La fuente «de cualquier equipo» de una clase tipográfica.
fn del_sistema(c: gotv_diseno::recursos::Clase) -> &'static str {
    match c {
        gotv_diseno::recursos::Clase::Serif => "Georgia",
        gotv_diseno::recursos::Clase::Sans => "Arial",
        gotv_diseno::recursos::Clase::Mono => "Courier New",
    }
}

fn de_marca(marca: &str, t: &Value) -> TemaDoc {
    let texto_de = |k: &str, d: &str| t[k].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(d).to_string();
    let par = texto_de("fuentes", "tecnico");
    let pedido = gotv_diseno::tema::Pedido { marca: marca.to_string(), croma: texto_de("croma", "medio"), fuentes: par.clone(), ..Default::default() };
    let tema = gotv_diseno::tema::Tema::new(pedido);
    let c = |k: &str| tema.claro.colores.get(k).map(|h| limpio(h)).unwrap_or_else(|| "1F2937".into());
    let (ft, fc) = gotv_diseno::recursos::par(&par).or_else(|| gotv_diseno::recursos::par("tecnico")).map(|(a, b)| (del_sistema(a.clase), del_sistema(b.clase))).unwrap_or(("Arial", "Arial"));
    TemaDoc {
        primario: c("acento"),
        sobre_primario: c("sobre-acento"),
        titulo: c("acento-texto"),
        texto: c("texto"),
        tenue: c("texto-tenue"),
        suave: c("acento-suave"),
        borde: c("borde-fuerte"),
        fuente_titulo: ft.into(),
        fuente_cuerpo: fc.into(),
        par,
    }
}

fn de_paleta(base: [&str; 3]) -> TemaDoc {
    let primario = base[0].to_string();
    TemaDoc {
        sobre_primario: crate::ppt::legible("FFFFFF", &primario),
        titulo: crate::ppt::legible(&primario, "FFFFFF"),
        primario,
        texto: "1F2937".into(),
        tenue: "4B5563".into(),
        suave: "F3F4F6".into(),
        borde: "9CA3AF".into(),
        fuente_titulo: "Georgia".into(),
        fuente_cuerpo: "Arial".into(),
        par: String::new(),
    }
}

fn paleta(nombre: &str) -> Option<[&'static str; 3]> {
    let n = nombre.trim().to_lowercase();
    (!n.is_empty()).then(|| crate::ppt::PALETAS.iter().find(|(p, _)| n == *p).map(|(_, c)| *c)).flatten()
}

/// El tema de la ficha (`ficha["tema"]`); `None` si no pide ninguno (el documento sale sobrio, como antes).
pub fn de(t: &Value) -> Option<TemaDoc> {
    let mut tema = match t {
        Value::String(s) if !s.trim().is_empty() => paleta(s).map(de_paleta).unwrap_or_else(|| de_marca(s.trim(), &Value::Null)),
        Value::Object(o) if !o.is_empty() => {
            let s = |k: &str| t[k].as_str().map(str::trim).filter(|x| !x.is_empty());
            match (s("paleta").and_then(paleta), s("marca").or(s("color"))) {
                (Some(p), None) => de_paleta(p),
                (_, Some(m)) => de_marca(m, t),
                // sólo colores o fuentes sueltos: sobre la marca por omisión
                (None, None) => de_marca(s("primario").unwrap_or("azul"), t),
            }
        }
        _ => return None,
    };
    if let Some(p) = t["primario"].as_str().and_then(crate::ppt::hex) {
        tema.sobre_primario = crate::ppt::legible("FFFFFF", &p);
        tema.titulo = crate::ppt::legible(&p, "FFFFFF");
        tema.primario = p;
    }
    for (k, campo) in [("fuente_titulo", &mut tema.fuente_titulo), ("fuente_cuerpo", &mut tema.fuente_cuerpo)] {
        if let Some(f) = t[k].as_str().map(str::trim).filter(|f| !f.is_empty()) {
            *campo = f.to_string();
        }
    }
    Some(tema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contraste(a: &str, b: &str) -> f64 {
        crate::ppt::contraste(a, b)
    }

    /// Venga de donde venga, el tema se LEE: títulos y texto sobre blanco, y el texto de la cabecera
    /// sobre el color de marca, todos con contraste AA o más.
    #[test]
    fn todo_tema_se_lee() {
        let pedidos = [
            json!("azul"),
            json!("#7c3aed"),
            json!("amarillo"),
            json!("gris"),
            json!({"marca": "verde", "croma": "vivo", "fuentes": "editorial"}),
            json!({"marca": "#f9e795"}),
            json!({"paleta": "midnight"}),
            json!("coral"),
            json!("cereza"),
            json!({"paleta": "salvia"}),
            json!({"primario": "#FFE600"}),
        ];
        for p in pedidos {
            let t = de(&p).unwrap_or_else(|| panic!("{p}"));
            for (que, frente, fondo, min) in [
                ("título", &t.titulo, "FFFFFF", 4.5),
                ("texto", &t.texto, "FFFFFF", 7.0),
                ("tenue", &t.tenue, "FFFFFF", 4.5),
                ("cabecera", &t.sobre_primario, t.primario.as_str(), 4.5),
                ("texto sobre fila alterna", &t.texto, t.suave.as_str(), 7.0),
            ] {
                let c = contraste(frente, fondo);
                assert!(c >= min, "{p}: {que} {frente} sobre {fondo} = {c:.2} (mínimo {min})");
            }
            for h in [&t.primario, &t.sobre_primario, &t.titulo, &t.texto, &t.tenue, &t.suave, &t.borde] {
                assert!(h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()), "{p}: «{h}» no es un hex de 6");
            }
        }
    }

    /// Sin tema no hay tema; una paleta de las presentaciones da SU color; el par tipográfico se
    /// traduce a fuentes del sistema y las fuentes dichas a mano mandan.
    #[test]
    fn de_donde_sale_cada_cosa() {
        assert_eq!(de(&Value::Null), None);
        assert_eq!(de(&json!("")), None);
        assert_eq!(de(&json!({})), None);
        assert_eq!(de(&json!("midnight")).unwrap().primario, "1E2761");
        assert_eq!(de(&json!({"paleta": "oceano"})).unwrap().primario, "065A82");
        // una marca gana a una paleta si vienen las dos
        assert_ne!(de(&json!({"paleta": "oceano", "marca": "rojo"})).unwrap().primario, "065A82");
        let e = de(&json!({"marca": "azul", "fuentes": "editorial"})).unwrap();
        assert_eq!((e.fuente_titulo.as_str(), e.par.as_str()), ("Georgia", "editorial"));
        let t = de(&json!({"marca": "azul"})).unwrap();
        assert_eq!((t.fuente_titulo.as_str(), t.fuente_cuerpo.as_str()), ("Arial", "Arial"));
        let m = de(&json!({"marca": "azul", "fuente_titulo": "Garamond", "primario": "#0F766E"})).unwrap();
        assert_eq!((m.fuente_titulo.as_str(), m.primario.as_str()), ("Garamond", "0F766E"));
        // el mismo pedido, el mismo tema (sin azar)
        assert_eq!(de(&json!("morado")), de(&json!("morado")));
    }
}
