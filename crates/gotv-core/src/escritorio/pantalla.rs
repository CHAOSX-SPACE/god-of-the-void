//! F16.7 · La pantalla de la terminal del usuario, emulada en Rust (regla 1 del Portador: la lógica no va en
//! el JavaScript, que sólo pinta filas). Un VT100/xterm mínimo: texto con ajuste de línea, retorno, tabulador,
//! movimientos del cursor, borrados, inserción y borrado de líneas y caracteres, región de desplazamiento,
//! colores (16, 256 y 24 bits), negrita/subrayado/inverso, pantalla alterna (vim, less, top), guardar y
//! restaurar el cursor, títulos (OSC, se ignoran) y las preguntas que una shell hace a la terminal (posición
//! del cursor, atributos): sus respuestas quedan en [`Pantalla::respuestas`] para devolverlas al pty.
use serde_json::{json, Value};
use std::collections::VecDeque;

/// Líneas que se recuerdan por encima de la pantalla.
pub const HISTORIAL: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Defecto,
    Indice(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    fn json(self) -> Value {
        match self {
            Color::Defecto => Value::Null,
            Color::Indice(i) => json!(i),
            Color::Rgb(r, g, b) => json!(format!("#{r:02x}{g:02x}{b:02x}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Estilo {
    pub fg: Color,
    pub bg: Color,
    pub negrita: bool,
    pub tenue: bool,
    pub cursiva: bool,
    pub subrayado: bool,
    pub inverso: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Celda {
    pub c: char,
    pub e: Estilo,
}

impl Default for Celda {
    fn default() -> Self {
        Celda { c: ' ', e: Estilo::default() }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Parser {
    Normal,
    Esc,
    /// ESC ( o ESC ) …: se come un carácter (juego de caracteres)
    Charset,
    Csi(String),
    Osc(String),
    OscEsc(String),
}

pub struct Pantalla {
    pub filas: usize,
    pub cols: usize,
    lineas: Vec<Vec<Celda>>,
    historial: VecDeque<Vec<Celda>>,
    /// (fila, columna)
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    estilo: Estilo,
    guardado: ((usize, usize), Estilo),
    arriba: usize,
    abajo: usize,
    /// el cursor llegó al final de la línea: el próximo carácter salta a la siguiente
    ajuste_pendiente: bool,
    alterna: Option<(Vec<Vec<Celda>>, (usize, usize))>,
    parser: Parser,
    utf8: Vec<u8>,
    /// lo que la terminal le contesta a la shell (DSR, DA): se escribe de vuelta al pty
    pub respuestas: Vec<u8>,
    /// cambió algo desde la última vista
    pub sucia: bool,
}

impl Pantalla {
    pub fn new(filas: usize, cols: usize) -> Pantalla {
        let (filas, cols) = (filas.clamp(2, 500), cols.clamp(10, 1000));
        Pantalla {
            filas,
            cols,
            lineas: vec![vec![Celda::default(); cols]; filas],
            historial: VecDeque::new(),
            cursor: (0, 0),
            cursor_visible: true,
            estilo: Estilo::default(),
            guardado: ((0, 0), Estilo::default()),
            arriba: 0,
            abajo: filas - 1,
            ajuste_pendiente: false,
            alterna: None,
            parser: Parser::Normal,
            utf8: Vec::new(),
            respuestas: Vec::new(),
            sucia: true,
        }
    }

    pub fn redimensionar(&mut self, filas: usize, cols: usize) {
        let (filas, cols) = (filas.clamp(2, 500), cols.clamp(10, 1000));
        for l in self.lineas.iter_mut() {
            l.resize(cols, Celda::default());
        }
        while self.lineas.len() > filas {
            // lo que sobra arriba pasa al historial (si no es la pantalla alterna)
            let l = self.lineas.remove(0);
            if self.alterna.is_none() {
                self.empujar_historial(l);
            }
            self.cursor.0 = self.cursor.0.saturating_sub(1);
        }
        while self.lineas.len() < filas {
            self.lineas.push(vec![Celda::default(); cols]);
        }
        self.filas = filas;
        self.cols = cols;
        self.arriba = 0;
        self.abajo = filas - 1;
        self.cursor = (self.cursor.0.min(filas - 1), self.cursor.1.min(cols - 1));
        self.sucia = true;
    }

    fn empujar_historial(&mut self, l: Vec<Celda>) {
        self.historial.push_back(l);
        while self.historial.len() > HISTORIAL {
            self.historial.pop_front();
        }
    }

    pub fn alimentar(&mut self, datos: &[u8]) {
        self.sucia = true;
        let mut b = std::mem::take(&mut self.utf8);
        b.extend_from_slice(datos);
        let mut i = 0;
        while i < b.len() {
            let x = b[i];
            let largo = match x {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => {
                    i += 1;
                    continue;
                }
            };
            if i + largo > b.len() {
                self.utf8 = b[i..].to_vec();
                return;
            }
            match std::str::from_utf8(&b[i..i + largo]) {
                Ok(s) => {
                    for c in s.chars() {
                        self.caracter(c);
                    }
                }
                Err(_) => self.caracter('\u{fffd}'),
            }
            i += largo;
        }
    }

    fn caracter(&mut self, c: char) {
        match std::mem::replace(&mut self.parser, Parser::Normal) {
            Parser::Normal => self.normal(c),
            Parser::Esc => self.escape(c),
            Parser::Charset => {}
            Parser::Csi(mut p) => {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    self.csi(&p, c);
                } else if p.len() < 64 {
                    p.push(c);
                    self.parser = Parser::Csi(p);
                }
            }
            Parser::Osc(mut p) => match c {
                '\u{7}' => {}
                '\u{1b}' => self.parser = Parser::OscEsc(p),
                _ => {
                    if p.len() < 512 {
                        p.push(c);
                    }
                    self.parser = Parser::Osc(p);
                }
            },
            Parser::OscEsc(p) => {
                if c != '\\' {
                    self.parser = Parser::Osc(p);
                }
            }
        }
    }

    fn normal(&mut self, c: char) {
        match c {
            '\u{1b}' => self.parser = Parser::Esc,
            '\r' => {
                self.cursor.1 = 0;
                self.ajuste_pendiente = false;
            }
            '\n' | '\u{b}' | '\u{c}' => self.salto(),
            '\u{8}' => {
                self.cursor.1 = self.cursor.1.saturating_sub(1);
                self.ajuste_pendiente = false;
            }
            '\t' => {
                self.cursor.1 = ((self.cursor.1 / 8 + 1) * 8).min(self.cols - 1);
            }
            '\u{7}' | '\u{0}' | '\u{e}' | '\u{f}' => {}
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => self.escribir(c),
        }
    }

    fn escribir(&mut self, c: char) {
        if self.ajuste_pendiente {
            self.ajuste_pendiente = false;
            self.cursor.1 = 0;
            self.salto();
        }
        let (f, col) = self.cursor;
        self.lineas[f][col] = Celda { c, e: self.estilo };
        if col + 1 >= self.cols {
            self.ajuste_pendiente = true;
        } else {
            self.cursor.1 += 1;
        }
    }

    /// Bajar una línea (desplazando la región si hace falta).
    fn salto(&mut self) {
        self.ajuste_pendiente = false;
        if self.cursor.0 == self.abajo {
            self.desplazar_arriba(1);
        } else if self.cursor.0 + 1 < self.filas {
            self.cursor.0 += 1;
        }
    }

    fn desplazar_arriba(&mut self, n: usize) {
        for _ in 0..n {
            let l = self.lineas.remove(self.arriba);
            if self.arriba == 0 && self.alterna.is_none() {
                self.empujar_historial(l);
            }
            self.lineas.insert(self.abajo, vec![Celda { c: ' ', e: Estilo { bg: self.estilo.bg, ..Default::default() } }; self.cols]);
        }
    }

    fn desplazar_abajo(&mut self, n: usize) {
        for _ in 0..n {
            self.lineas.remove(self.abajo);
            self.lineas.insert(self.arriba, vec![Celda::default(); self.cols]);
        }
    }

    fn escape(&mut self, c: char) {
        match c {
            '[' => self.parser = Parser::Csi(String::new()),
            ']' => self.parser = Parser::Osc(String::new()),
            '(' | ')' | '*' | '+' => self.parser = Parser::Charset,
            '7' => self.guardado = (self.cursor, self.estilo),
            '8' => {
                (self.cursor, self.estilo) = self.guardado;
                self.ajuste_pendiente = false;
            }
            'D' => self.salto(),
            'E' => {
                self.cursor.1 = 0;
                self.salto();
            }
            'M' => {
                if self.cursor.0 == self.arriba {
                    self.desplazar_abajo(1);
                } else {
                    self.cursor.0 = self.cursor.0.saturating_sub(1);
                }
            }
            'c' => {
                let (f, c) = (self.filas, self.cols);
                let h = std::mem::take(&mut self.historial);
                *self = Pantalla::new(f, c);
                self.historial = h;
            }
            _ => {}
        }
    }

    fn borrar(&mut self, f: usize, desde: usize, hasta: usize) {
        let vacia = Celda { c: ' ', e: Estilo { bg: self.estilo.bg, ..Default::default() } };
        for x in self.lineas[f][desde.min(self.cols)..hasta.min(self.cols)].iter_mut() {
            *x = vacia;
        }
    }

    fn csi(&mut self, p: &str, fin: char) {
        let privado = p.starts_with('?');
        let ps: Vec<usize> = p.trim_start_matches(['?', '>', '=']).split(';').map(|x| x.parse().unwrap_or(0)).collect();
        let n = |i: usize| ps.get(i).copied().filter(|&x| x > 0).unwrap_or(1);
        let (f, c) = self.cursor;
        self.ajuste_pendiente = false;
        match fin {
            'A' => self.cursor.0 = f.saturating_sub(n(0)).max(if f >= self.arriba { self.arriba } else { 0 }),
            'B' | 'e' => self.cursor.0 = (f + n(0)).min(if f <= self.abajo { self.abajo } else { self.filas - 1 }),
            'C' | 'a' => self.cursor.1 = (c + n(0)).min(self.cols - 1),
            'D' => self.cursor.1 = c.saturating_sub(n(0)),
            'E' => self.cursor = ((f + n(0)).min(self.filas - 1), 0),
            'F' => self.cursor = (f.saturating_sub(n(0)), 0),
            'G' | '`' => self.cursor.1 = (n(0) - 1).min(self.cols - 1),
            'd' => self.cursor.0 = (n(0) - 1).min(self.filas - 1),
            'H' | 'f' => self.cursor = ((n(0) - 1).min(self.filas - 1), (n(1) - 1).min(self.cols - 1)),
            'J' => match ps.first().copied().unwrap_or(0) {
                0 => {
                    self.borrar(f, c, self.cols);
                    for x in f + 1..self.filas {
                        self.borrar(x, 0, self.cols);
                    }
                }
                1 => {
                    for x in 0..f {
                        self.borrar(x, 0, self.cols);
                    }
                    self.borrar(f, 0, c + 1);
                }
                2 => {
                    for x in 0..self.filas {
                        self.borrar(x, 0, self.cols);
                    }
                }
                3 => self.historial.clear(),
                _ => {}
            },
            'K' => match ps.first().copied().unwrap_or(0) {
                0 => self.borrar(f, c, self.cols),
                1 => self.borrar(f, 0, c + 1),
                _ => self.borrar(f, 0, self.cols),
            },
            'L' if (self.arriba..=self.abajo).contains(&f) => {
                for _ in 0..n(0).min(self.abajo - f + 1) {
                    self.lineas.remove(self.abajo);
                    self.lineas.insert(f, vec![Celda::default(); self.cols]);
                }
            }
            'M' if (self.arriba..=self.abajo).contains(&f) => {
                for _ in 0..n(0).min(self.abajo - f + 1) {
                    self.lineas.remove(f);
                    self.lineas.insert(self.abajo, vec![Celda::default(); self.cols]);
                }
            }
            'P' => {
                let l = &mut self.lineas[f];
                for _ in 0..n(0).min(self.cols - c) {
                    l.remove(c);
                    l.push(Celda::default());
                }
            }
            '@' => {
                let l = &mut self.lineas[f];
                for _ in 0..n(0).min(self.cols - c) {
                    l.insert(c, Celda::default());
                    l.pop();
                }
            }
            'X' => self.borrar(f, c, c + n(0)),
            'S' => self.desplazar_arriba(n(0)),
            'T' => self.desplazar_abajo(n(0)),
            'r' => {
                let a = n(0) - 1;
                let b = ps.get(1).copied().filter(|&x| x > 0).unwrap_or(self.filas) - 1;
                if a < b && b < self.filas {
                    (self.arriba, self.abajo) = (a, b);
                    self.cursor = (0, 0);
                }
            }
            's' => self.guardado = (self.cursor, self.estilo),
            'u' => (self.cursor, self.estilo) = self.guardado,
            'm' => self.sgr(&ps),
            'n' if ps.first() == Some(&6) => self.respuestas.extend(format!("\x1b[{};{}R", f + 1, c + 1).bytes()),
            'n' if ps.first() == Some(&5) => self.respuestas.extend(b"\x1b[0n"),
            'c' if !privado => self.respuestas.extend(b"\x1b[?1;2c"),
            'h' | 'l' if privado => {
                let on = fin == 'h';
                for m in &ps {
                    match m {
                        25 => self.cursor_visible = on,
                        47 | 1047 | 1049 => self.pantalla_alterna(on),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn pantalla_alterna(&mut self, on: bool) {
        if on && self.alterna.is_none() {
            let vacia = vec![vec![Celda::default(); self.cols]; self.filas];
            self.alterna = Some((std::mem::replace(&mut self.lineas, vacia), self.cursor));
        } else if !on {
            if let Some((l, cur)) = self.alterna.take() {
                self.lineas = l;
                self.cursor = cur;
            }
        }
    }

    fn sgr(&mut self, ps: &[usize]) {
        let mut i = 0;
        while i < ps.len() {
            let e = &mut self.estilo;
            match ps[i] {
                0 => *e = Estilo::default(),
                1 => e.negrita = true,
                2 => e.tenue = true,
                3 => e.cursiva = true,
                4 => e.subrayado = true,
                7 => e.inverso = true,
                21 | 22 => (e.negrita, e.tenue) = (false, false),
                23 => e.cursiva = false,
                24 => e.subrayado = false,
                27 => e.inverso = false,
                x @ 30..=37 => e.fg = Color::Indice((x - 30) as u8),
                39 => e.fg = Color::Defecto,
                x @ 40..=47 => e.bg = Color::Indice((x - 40) as u8),
                49 => e.bg = Color::Defecto,
                x @ 90..=97 => e.fg = Color::Indice((x - 90 + 8) as u8),
                x @ 100..=107 => e.bg = Color::Indice((x - 100 + 8) as u8),
                k @ (38 | 48) => {
                    let color = match ps.get(i + 1) {
                        Some(5) => {
                            i += 2;
                            ps.get(i).map(|&x| Color::Indice(x.min(255) as u8))
                        }
                        Some(2) => {
                            i += 4;
                            match (ps.get(i - 2), ps.get(i - 1), ps.get(i)) {
                                (Some(&r), Some(&g), Some(&b)) => Some(Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(c) = color {
                        if k == 38 {
                            e.fg = c
                        } else {
                            e.bg = c
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// La pantalla para pintar: filas como tramos de igual estilo, y el cursor.
    pub fn vista(&mut self) -> Value {
        self.sucia = false;
        let filas: Vec<Value> = self.lineas.iter().map(|l| tramos(l)).collect();
        json!({"filas": filas, "cursor": [self.cursor.0, self.cursor.1], "cursor_visible": self.cursor_visible, "alterna": self.alterna.is_some(), "cols": self.cols})
    }

    /// Texto plano: las últimas `n` líneas (historial + pantalla), sin espacios de relleno ni líneas vacías al final.
    pub fn texto(&self, n: usize) -> String {
        let todas: Vec<String> = self.historial.iter().chain(self.lineas.iter()).map(|l| l.iter().map(|c| c.c).collect::<String>().trim_end().to_string()).collect();
        let mut fin = todas.len();
        while fin > 0 && todas[fin - 1].is_empty() {
            fin -= 1;
        }
        todas[fin.saturating_sub(n)..fin].join("\n")
    }
}

fn tramos(l: &[Celda]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut t = String::new();
    let mut e: Option<Estilo> = None;
    let cerrar = |t: &mut String, e: Option<Estilo>, out: &mut Vec<Value>| {
        if t.is_empty() {
            return;
        }
        let e = e.unwrap_or_default();
        let mut v = json!({"t": std::mem::take(t)});
        if e != Estilo::default() {
            let (fg, bg) = if e.inverso { (e.bg, e.fg) } else { (e.fg, e.bg) };
            v["fg"] = fg.json();
            v["bg"] = bg.json();
            if e.inverso {
                v["inv"] = json!(true);
            }
            if e.negrita {
                v["b"] = json!(true);
            }
            if e.tenue {
                v["d"] = json!(true);
            }
            if e.cursiva {
                v["i"] = json!(true);
            }
            if e.subrayado {
                v["u"] = json!(true);
            }
        }
        out.push(v);
    };
    // los espacios por defecto del final no viajan
    let usado = l.iter().rposition(|c| c.c != ' ' || c.e != Estilo::default()).map(|i| i + 1).unwrap_or(0);
    for c in &l[..usado] {
        if e != Some(c.e) {
            cerrar(&mut t, e, &mut out);
            e = Some(c.e);
        }
        t.push(c.c);
    }
    cerrar(&mut t, e, &mut out);
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fila(p: &mut Pantalla, i: usize) -> String {
        p.vista()["filas"][i].as_array().unwrap().iter().map(|x| x["t"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn texto_salto_ajuste_y_retroceso() {
        let mut p = Pantalla::new(4, 10);
        p.alimentar(b"hola\r\nmundo");
        assert_eq!((fila(&mut p, 0), fila(&mut p, 1)), ("hola".into(), "mundo".into()));
        assert_eq!(p.cursor, (1, 5));
        p.alimentar(b"\x08\x08XY");
        assert_eq!(fila(&mut p, 1), "munXY");
        p.alimentar(b"\r\n0123456789abc");
        assert_eq!((fila(&mut p, 2), fila(&mut p, 3)), ("0123456789".into(), "abc".into()), "ajuste al final de la línea");
        p.alimentar(b"\r\nuna\r\ndos");
        assert_eq!(fila(&mut p, 3), "dos");
        assert!(p.texto(10).starts_with("hola\nmunXY"), "lo que sube va al historial: {}", p.texto(10));
        p.alimentar("ñandú ✓".as_bytes());
        assert!(p.texto(1).ends_with("dosñandú ✓"), "{}", p.texto(1));
    }

    #[test]
    fn utf8_partido_entre_lecturas() {
        let mut p = Pantalla::new(2, 10);
        let b = "ñ".as_bytes();
        p.alimentar(&b[..1]);
        p.alimentar(&b[1..]);
        assert_eq!(fila(&mut p, 0), "ñ");
    }

    #[test]
    fn cursor_borrados_colores_y_alterna() {
        let mut p = Pantalla::new(5, 20);
        p.alimentar(b"\x1b[2J\x1b[3;5Hx\x1b[1;31mrojo\x1b[0m");
        assert_eq!(p.cursor, (2, 9));
        let v = p.vista();
        let tr = v["filas"][2].as_array().unwrap();
        assert_eq!(tr[1]["t"], "rojo");
        assert_eq!((tr[1]["fg"].clone(), tr[1]["b"].clone()), (json!(1), json!(true)));
        p.alimentar(b"\x1b[38;2;255;128;0mnaranja\x1b[38;5;200m!");
        let v = p.vista();
        assert!(v["filas"][2].as_array().unwrap().iter().any(|t| t["fg"] == "#ff8000"));
        p.alimentar(b"\x1b[3;1H\x1b[K");
        assert_eq!(fila(&mut p, 2), "");
        // pantalla alterna: lo de antes vuelve intacto
        p.alimentar(b"\x1b[1;1Hbase");
        p.alimentar(b"\x1b[?1049h\x1b[1;1Hvim");
        assert_eq!(fila(&mut p, 0), "vim");
        p.alimentar(b"\x1b[?1049l");
        assert_eq!(fila(&mut p, 0), "base");
        // la shell pregunta dónde está el cursor
        p.alimentar(b"\x1b[6n");
        assert_eq!(p.respuestas, b"\x1b[1;5R");
        p.alimentar(b"\x1b]0;titulo\x07ok");
        assert_eq!(fila(&mut p, 0), "baseok");
    }

    #[test]
    fn region_de_desplazamiento_y_redimensionar() {
        let mut p = Pantalla::new(4, 10);
        p.alimentar(b"a\r\nb\r\nc\r\nd");
        p.alimentar(b"\x1b[2;3r\x1b[3;1H\n");
        assert_eq!((fila(&mut p, 0), fila(&mut p, 1), fila(&mut p, 2), fila(&mut p, 3)), ("a".into(), "c".into(), "".into(), "d".into()));
        p.redimensionar(2, 12);
        assert_eq!(p.filas, 2);
        assert_eq!(p.cols, 12);
    }
}
