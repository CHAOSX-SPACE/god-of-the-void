//! M2 · leer en voz alta: de la respuesta en markdown al texto que se DICE, partido en trozos que
//! caben en una pasada de la voz (la tabla de posiciones del talker y el ritmo del habla).
//!
//! Se dice la prosa: los bloques de código y las tablas se omiten (se anuncian una vez), `código` en
//! línea y los enlaces quedan como su texto, se quitan las marcas (#, *, _, >, viñetas) y cada
//! título o viñeta se cierra con punto (una pausa al oírlo).

/// Caracteres por trozo: ~25 s de habla, lejos del tope de la voz (2 000 fotogramas, 160 s).
pub const TROZO_MAX: usize = 320;

/// El texto que se dice de un markdown, en trozos por frase (≤ `TROZO_MAX` caracteres).
pub fn trozos(md: &str, es: bool) -> Vec<String> {
    // la ubicación y la licencia de lo generado no se dicen (viven en la tarjeta)
    let md = &crate::mensajes::sin_ubicaciones(md);
    let mut limpio = String::new();
    let mut en_codigo = false;
    let mut codigo_dicho = false;
    let mut tabla_dicha = false;
    for linea in md.lines() {
        let l = linea.trim();
        if l.starts_with("```") || l.starts_with("~~~") {
            en_codigo = !en_codigo;
            if en_codigo && !codigo_dicho {
                limpio.push_str(if es { " (ahí va un bloque de código). " } else { " (here is a code block). " }); // juez-idioma: par (if es)
                codigo_dicho = true;
            }
            continue;
        }
        if en_codigo || l.is_empty() || l.chars().all(|c| matches!(c, '-' | '|' | ':' | ' ' | '=' | '*' | '_')) {
            if l.is_empty() {
                limpio.push_str(". ");
            }
            continue;
        }
        // la línea del sello («🕳️ …», el cierre que el modelo imita de la skill CHAOS) no se dice
        if l.starts_with('🕳') {
            continue;
        }
        if l.starts_with('|') {
            if !tabla_dicha {
                limpio.push_str(if es { " (ahí va una tabla). " } else { " (here is a table). " }); // juez-idioma: par (if es)
                tabla_dicha = true;
            }
            continue;
        }
        let titulo = l.starts_with('#');
        let l = l.trim_start_matches(['#', '>', ' ']);
        let sin = quitar_vineta(l);
        let vineta = sin.len() != l.trim_start().len();
        let mut t = en_linea(sin, es).trim_end().to_string();
        if (titulo || vineta) && !t.ends_with(['.', '!', '?', ':', ';']) {
            t.push('.');
        }
        limpio.push_str(&t);
        limpio.push(' ');
    }
    partir(&normalizar(&limpio))
}

fn quitar_vineta(l: &str) -> &str {
    let t = l.trim_start();
    for p in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(p) {
            return r;
        }
    }
    // «1. » / «12) »
    let d = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if d > 0 && d < 4 {
        let r = &t[d..];
        if let Some(r) = r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")) {
            return r;
        }
    }
    t
}

/// Enlaces → su texto; quita `*`, `_` de énfasis, comillas invertidas y barras de tabla.
fn en_linea(l: &str, es: bool) -> String {
    let mut s = String::with_capacity(l.len());
    let c: Vec<char> = l.chars().collect();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            // ![alt](url) o [texto](url) → texto
            '!' if c.get(i + 1) == Some(&'[') => i += 1,
            '[' => {
                if let Some(cierre) = c[i..].iter().position(|&x| x == ']').map(|p| p + i) {
                    if c.get(cierre + 1) == Some(&'(') {
                        if let Some(fin) = c[cierre..].iter().position(|&x| x == ')').map(|p| p + cierre) {
                            s.extend(&c[i + 1..cierre]);
                            i = fin + 1;
                            continue;
                        }
                    }
                }
                i += 1;
            }
            // `código en línea`: un nombre corto (main.rs) se dice; un COMANDO (con espacios o largo) no
            '`' => {
                match c[i + 1..].iter().position(|&x| x == '`').map(|p| p + i + 1) {
                    Some(fin) => {
                        let dentro: String = c[i + 1..fin].iter().collect();
                        if dentro.trim().contains(char::is_whitespace) || dentro.chars().count() > 30 {
                            s.push_str(if es { "el comando del chat" } else { "the command in the chat" }); // juez-idioma: par (if es)
                        } else {
                            s.push_str(&dentro);
                        }
                        i = fin + 1;
                    }
                    None => i += 1,
                }
            }
            '*' => i += 1,
            // `_` de énfasis (no el de dentro de una palabra: nombres_de_archivo)
            '_' if i == 0 || !c[i - 1].is_alphanumeric() || !c.get(i + 1).is_some_and(|x| x.is_alphanumeric()) => i += 1,
            '|' => {
                s.push_str(", ");
                i += 1;
            }
            // los emojis no se dicen (la voz los pronuncia mal o como ruido)
            x if es_emoji(x) => i += 1,
            x => {
                s.push(x);
                i += 1;
            }
        }
    }
    s
}

fn es_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE0F | 0x200D)
}

fn normalizar(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for p in s.split_whitespace() {
        r.push_str(p);
        r.push(' ');
    }
    // puntos repetidos por las líneas vacías: «. .» → «.»
    let mut r = r.trim().to_string();
    for (a, b) in [(" .", "."), ("..", "."), (". .", "."), (":.", ":"), (",.", "."), ("!.", "!"), ("?.", "?")] {
        while r.contains(a) {
            r = r.replace(a, b);
        }
    }
    r.trim_start_matches(['.', ' ']).trim().to_string()
}

/// Parte en frases y las junta hasta `TROZO_MAX`; una frase más larga se corta por comas o espacios.
fn partir(s: &str) -> Vec<String> {
    let mut frases: Vec<String> = Vec::new();
    let mut actual = String::new();
    let c: Vec<char> = s.chars().collect();
    for (i, &ch) in c.iter().enumerate() {
        actual.push(ch);
        // fin de frase = signo + espacio o fin (no el punto de «main.rs» ni de «3.5»)
        if matches!(ch, '.' | '!' | '?' | ';' | ':') && c.get(i + 1).is_none_or(|x| x.is_whitespace()) {
            frases.push(std::mem::take(&mut actual));
        }
    }
    frases.push(actual);
    let mut out: Vec<String> = Vec::new();
    for f in frases.into_iter().map(|f| f.trim().to_string()).filter(|f| f.chars().any(|c| c.is_alphanumeric())) {
        for pedazo in cortar_largo(&f) {
            match out.last_mut() {
                Some(u) if u.chars().count() + 1 + pedazo.chars().count() <= TROZO_MAX => {
                    u.push(' ');
                    u.push_str(&pedazo);
                }
                _ => out.push(pedazo),
            }
        }
    }
    out
}

fn cortar_largo(f: &str) -> Vec<String> {
    if f.chars().count() <= TROZO_MAX {
        return vec![f.to_string()];
    }
    let mut v = Vec::new();
    let mut actual = String::new();
    for palabra in f.split(' ') {
        if actual.chars().count() + palabra.chars().count() + 1 > TROZO_MAX && !actual.is_empty() {
            v.push(std::mem::take(&mut actual));
        }
        if !actual.is_empty() {
            actual.push(' ');
        }
        actual.push_str(palabra);
    }
    if !actual.is_empty() {
        v.push(actual);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dice_la_prosa_y_salta_el_codigo() {
        let md = "## Hecho\n\nArreglé **la ruta** en `main.rs` y [la guía](https://x.y/z).\n\n```rust\nfn main() {}\n```\n\n- primero\n- segundo_paso\n\n| a | b |\n|---|---|\n| 1 | 2 |";
        let t = trozos(md, true);
        assert_eq!(t.len(), 1, "{t:?}");
        let s = &t[0];
        assert_eq!(s, "Hecho. Arreglé la ruta en main.rs y la guía. (ahí va un bloque de código). primero. segundo_paso. (ahí va una tabla).");
        assert!(s.contains("(ahí va un bloque de código)"));
        assert!(!s.contains("fn main"));
        assert!(s.contains("primero") && s.contains("segundo_paso"));
        assert!(!s.contains('*') && !s.contains('`') && !s.contains('#') && !s.contains("https"));
    }

    #[test]
    fn parte_lo_largo_en_trozos_que_caben() {
        let frase = "Esta es una frase de prueba con varias palabras para medir el corte. ";
        let t = trozos(&frase.repeat(30), true);
        assert!(t.len() > 3);
        assert!(t.iter().all(|x| x.chars().count() <= TROZO_MAX), "{t:?}");
        assert!(t.iter().all(|x| x.ends_with('.')), "se corta en fin de frase: {t:?}");
        let sin_puntos = "palabra ".repeat(200);
        assert!(trozos(&sin_puntos, true).iter().all(|x| x.chars().count() <= TROZO_MAX));
    }

    #[test]
    fn el_sello_y_los_emojis_no_se_leen() {
        let t = trozos("Listo, ya quedó ✅ funcionando 🚀.\n\n🕳️ Lo devorado no retorna. Lo que entregué, sobrevivió al Juicio.", true).join(" ");
        assert_eq!(t, "Listo, ya quedó funcionando.");
    }

    #[test]
    fn los_comandos_no_se_leen() {
        let t = trozos("Corre `cargo build --release` y abre `main.rs`.", true).join(" ");
        assert_eq!(t, "Corre el comando del chat y abre main.rs.");
    }

    /// El parlante y los manos libres no dicen dónde quedó la canción ni su licencia.
    #[test]
    fn la_ubicacion_no_se_lee() {
        let t = trozos("Listo, la canción suena alegre. La música está en `/Users/x/p/generado/musica-1.wav` (3.75 MB) y su licencia en `.licencia.txt`. ¿Le pongo voz?", true).join(" ");
        assert_eq!(t, "Listo, la canción suena alegre. ¿Le pongo voz?");
    }

    #[test]
    fn vacio_no_dice_nada() {
        assert!(trozos("", true).is_empty());
        assert!(trozos("```\nsolo código\n```", true).iter().all(|x| x.contains("código")));
        assert!(trozos("---\n\n***", true).is_empty());
    }
}
