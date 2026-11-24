//! F4 · El archivo `.ipynb`, tocado como lo dejaría Jupyter y SIN rehacer lo que no se toca.
//!
//! El notebook se lee entero con `serde_json` (validación y lectura) y, a la vez, se ubican en el TEXTO los
//! bytes de cada celda del arreglo `cells`. Editar es **empalmar**: sólo cambian los bytes de la celda tocada
//! (o los de la que entra o sale, con su coma); el resto del archivo —metadatos, campos que GOTV no conoce,
//! el formato de quien lo escribió— queda byte a byte. La celda nueva o cambiada sale como la escribe
//! nbformat: claves ordenadas, la sangría del propio archivo y el texto partido en líneas.
//!
//! Trampa 1 de la ficha: el formato 4.4 (y anteriores) no lleva `id` de celda: ahí se trabaja por índice y
//! no se inventa ninguno al guardar. Desde 4.5 el `id` es obligatorio: la celda nueva recibe uno (8 hex, como
//! nbformat). Trampa 5: si el archivo cambió en disco desde que se leyó (abierto en otro programa), no se pisa.

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Un notebook leído: su texto, su valor y dónde está cada celda en el texto.
pub struct Libro {
    pub ruta: PathBuf,
    pub texto: String,
    pub v: Value,
    /// `[` y `]` del arreglo `cells` (índices de byte, el `]` incluido)
    arreglo: (usize, usize),
    /// cada celda: desde su `{` hasta su `}` incluido
    celdas: Vec<(usize, usize)>,
    /// la sangría de una celda, la del `]` y la unidad (vacía si el archivo va en una sola línea)
    sangria_celda: String,
    cierre: String,
    unidad: String,
    /// fecha y tamaño al leerlo (trampa 5)
    huella: Option<(SystemTime, u64)>,
}

fn huella(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// El final (exclusivo) del valor JSON que empieza en `i` (ya sin blancos delante).
fn fin_valor(b: &[u8], i: usize) -> Result<usize> {
    let malo = || anyhow!("{}", crate::i18n::t("notebook: JSON incompleto", "notebook: incomplete JSON"));
    match b.get(i) {
        Some(b'"') => {
            let mut j = i + 1;
            while j < b.len() {
                match b[j] {
                    b'\\' => j += 2,
                    b'"' => return Ok(j + 1),
                    _ => j += 1,
                }
            }
            Err(malo())
        }
        Some(b'{' | b'[') => {
            let mut prof = 0i32;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = fin_valor(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => prof += 1,
                    b'}' | b']' => {
                        prof -= 1;
                        if prof == 0 {
                            return Ok(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            Err(malo())
        }
        Some(_) => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t') {
                j += 1;
            }
            Ok(j)
        }
        None => Err(malo()),
    }
}

fn blancos(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    i
}

/// Ubica el arreglo `cells` (de primer nivel) y sus elementos en el texto.
fn ubicar(texto: &str) -> Result<((usize, usize), Vec<(usize, usize)>)> {
    let b = texto.as_bytes();
    let mut i = blancos(b, 0);
    if b.get(i) != Some(&b'{') {
        bail!("{}", crate::i18n::t("notebook: no es un objeto JSON", "notebook: not a JSON object"));
    }
    i += 1;
    loop {
        i = blancos(b, i);
        match b.get(i) {
            Some(b'}') | None => break,
            Some(b',') => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let fin_clave = fin_valor(b, i)?;
        let clave: String = serde_json::from_str(&texto[i..fin_clave]).unwrap_or_default();
        i = blancos(b, fin_clave);
        if b.get(i) != Some(&b':') {
            bail!("{}", crate::i18n::t("notebook: falta «:»", "notebook: missing «:»"));
        }
        i = blancos(b, i + 1);
        let fin = fin_valor(b, i)?;
        if clave == "cells" && b.get(i) == Some(&b'[') {
            let mut celdas = Vec::new();
            let mut j = i + 1;
            loop {
                j = blancos(b, j);
                match b.get(j) {
                    Some(b']') => break,
                    Some(b',') => {
                        j += 1;
                        continue;
                    }
                    Some(_) => {
                        let f = fin_valor(b, j)?;
                        celdas.push((j, f - 1));
                        j = f;
                    }
                    None => bail!("{}", crate::i18n::t("notebook: celdas sin cerrar", "notebook: unclosed cells")),
                }
            }
            return Ok(((i, fin - 1), celdas));
        }
        i = fin;
    }
    bail!("{}", crate::i18n::t("notebook: no tiene «cells»", "notebook: it has no «cells»"))
}

/// Los blancos del principio de la línea en que está el byte `i`.
fn sangria_en(texto: &str, i: usize) -> Option<String> {
    let ini = texto[..i].rfind('\n')? + 1;
    let s = &texto[ini..i];
    s.chars().all(|c| c == ' ' || c == '\t').then(|| s.to_string())
}

/// Los blancos del principio de la línea en que está el byte `i` (haya lo que haya después).
fn sangria_linea(texto: &str, i: usize) -> String {
    let ini = texto[..i].rfind('\n').map(|k| k + 1).unwrap_or(0);
    texto[ini..i].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

impl Libro {
    pub fn leer(ruta: &Path) -> Result<Libro> {
        let texto = std::fs::read_to_string(ruta)?;
        Libro::de_texto(ruta, texto, huella(ruta))
    }

    fn de_texto(ruta: &Path, texto: String, huella: Option<(SystemTime, u64)>) -> Result<Libro> {
        let v: Value = serde_json::from_str(&texto)
            .map_err(|e| anyhow!("{}", crate::tr!("notebook inválido: {e}", "invalid notebook: {e}")))?;
        if !v["cells"].is_array() {
            bail!("{}", crate::i18n::t("notebook: no tiene «cells»", "notebook: it has no «cells»"));
        }
        let (arreglo, celdas) = ubicar(&texto)?;
        // la sangría: la de la primera celda (o la del `]` + una unidad); la unidad, la que separa el `]` del arreglo
        // de su primera celda (nbformat escribe con indent=1: el arreglo a 1 espacio y las celdas a 2)
        // un `"cells": []` en una línea de un archivo multilínea (el notebook recién creado): la sangría es la de
        // la línea de la clave, que está a un nivel
        let multilinea = texto.trim_end().contains('\n');
        let cierre = sangria_en(&texto, arreglo.1).or_else(|| (multilinea && celdas.is_empty()).then(|| sangria_linea(&texto, arreglo.0)));
        let primera = celdas.first().and_then(|c| sangria_en(&texto, c.0));
        let (sangria_celda, unidad) = match (&cierre, &primera) {
            (Some(c), Some(p)) if p.len() > c.len() => (p.clone(), p[c.len()..].to_string()),
            (Some(c), None) => (format!("{c}{}", if c.is_empty() { " " } else { c }), if c.is_empty() { " ".into() } else { c.clone() }),
            (_, Some(p)) => (p.clone(), " ".into()),
            _ => (String::new(), String::new()),
        };
        let cierre = cierre.unwrap_or_default();
        Ok(Libro { ruta: ruta.to_path_buf(), texto, v, arreglo, celdas, sangria_celda, cierre, unidad, huella })
    }

    pub fn celdas(&self) -> &[Value] {
        self.v["cells"].as_array().map(|a| a.as_slice()).unwrap_or(&[])
    }

    pub fn n(&self) -> usize {
        self.celdas.len()
    }

    /// nbformat 4.5 o más: las celdas llevan `id`.
    pub fn con_ids(&self) -> bool {
        let mayor = self.v["nbformat"].as_u64().unwrap_or(4);
        let menor = self.v["nbformat_minor"].as_u64().unwrap_or(0);
        mayor > 4 || (mayor == 4 && menor >= 5)
    }

    /// La celda pedida: un número (1 = la primera, como las muestra la lectura) o su `id`.
    pub fn indice(&self, celda: &Value) -> Result<usize> {
        let n = self.n();
        let por_numero = |k: u64| -> Result<usize> {
            if k >= 1 && (k as usize) <= n {
                Ok(k as usize - 1)
            } else {
                bail!("{}", crate::tr!("no hay celda {k}: el notebook tiene {n}", "there is no cell {k}: the notebook has {n}"))
            }
        };
        match celda {
            Value::Number(x) => por_numero(x.as_u64().unwrap_or(0)),
            Value::String(s) => {
                if let Some(i) = self.celdas().iter().position(|c| c["id"].as_str() == Some(s.as_str())) {
                    return Ok(i);
                }
                match s.trim().parse::<u64>() {
                    Ok(k) => por_numero(k),
                    Err(_) => bail!(
                        "{}",
                        crate::tr!(
                            "no hay celda con id «{s}» (este notebook {}): usa su número",
                            "there is no cell with id «{s}» (this notebook {}): use its number",
                            if self.con_ids() { crate::i18n::t("sí lleva ids", "does carry ids") } else { crate::i18n::t("es 4.4 o anterior: sin ids", "is 4.4 or older: no ids") }
                        )
                    ),
                }
            }
            _ => bail!("{}", crate::i18n::t("falta `celda` (su número, o su id)", "missing `celda` (its number, or its id)")),
        }
    }

    /// La celda como la escribe nbformat, a la sangría de las celdas de este archivo.
    fn texto_celda(&self, c: &Value) -> String {
        if self.unidad.is_empty() {
            return serde_json::to_string(&ordenado(c)).unwrap_or_default();
        }
        let mut o = String::new();
        escribir(&mut o, c, &self.unidad, self.sangria_celda.len() / self.unidad.len().max(1), &self.unidad);
        o
    }

    fn rehacer(&self, texto: String) -> Result<Libro> {
        // lo empalmado se vuelve a leer entero: si quedara mal, no se escribe
        Libro::de_texto(&self.ruta, texto, self.huella)
    }

    /// Cambia la celda `i` por `c` (sólo sus bytes).
    pub fn reemplazar(&self, i: usize, c: &Value) -> Result<Libro> {
        let (a, b) = self.celdas[i];
        let t = format!("{}{}{}", &self.texto[..a], self.texto_celda(c), &self.texto[b + 1..]);
        self.rehacer(t)
    }

    /// Mete `c` para que quede en la posición `i` (0 = al principio; `n` = al final).
    pub fn insertar(&self, i: usize, c: &Value) -> Result<Libro> {
        let tc = self.texto_celda(c);
        let sep = if self.unidad.is_empty() { ",".to_string() } else { format!(",\n{}", self.sangria_celda) };
        let t = if self.celdas.is_empty() {
            let (a, b) = self.arreglo;
            if self.unidad.is_empty() {
                format!("{}[{tc}]{}", &self.texto[..a], &self.texto[b + 1..])
            } else {
                format!("{}[\n{}{tc}\n{}]{}", &self.texto[..a], self.sangria_celda, self.cierre, &self.texto[b + 1..])
            }
        } else if i < self.celdas.len() {
            let a = self.celdas[i].0;
            format!("{}{tc}{sep}{}", &self.texto[..a], &self.texto[a..])
        } else {
            let b = self.celdas[self.celdas.len() - 1].1;
            format!("{}{sep}{tc}{}", &self.texto[..b + 1], &self.texto[b + 1..])
        };
        self.rehacer(t)
    }

    /// Quita la celda `i` (con su coma).
    pub fn borrar(&self, i: usize) -> Result<Libro> {
        let n = self.celdas.len();
        let t = if n == 1 {
            let (a, b) = self.arreglo;
            format!("{}[]{}", &self.texto[..a], &self.texto[b + 1..])
        } else if i + 1 < n {
            format!("{}{}", &self.texto[..self.celdas[i].0], &self.texto[self.celdas[i + 1].0..])
        } else {
            format!("{}{}", &self.texto[..self.celdas[i - 1].1 + 1], &self.texto[self.celdas[i].1 + 1..])
        };
        self.rehacer(t)
    }

    /// Una celda nueva como la crea nbformat (con `id` sólo si el formato lo lleva).
    pub fn celda_nueva(&self, tipo: &str, fuente: &str) -> Value {
        let mut m = Map::new();
        m.insert("cell_type".into(), Value::from(tipo));
        if self.con_ids() {
            m.insert("id".into(), Value::from(id_nuevo(self.celdas())));
        }
        m.insert("metadata".into(), Value::Object(Map::new()));
        m.insert("source".into(), lineas(fuente));
        if tipo == "code" {
            m.insert("execution_count".into(), Value::Null);
            m.insert("outputs".into(), Value::Array(vec![]));
        }
        Value::Object(m)
    }

    /// Escribe (atómico: temporal + rename, con los permisos del original). Si el archivo cambió en disco desde
    /// que se leyó, NO lo pisa (trampa 5).
    pub fn guardar(&self) -> Result<()> {
        if let (Some(antes), Some(ahora)) = (self.huella, huella(&self.ruta)) {
            if antes != ahora {
                bail!(
                    "{}",
                    crate::tr!(
                        "{} cambió en el disco desde que lo leí (¿está abierto en otro programa?): no lo piso. Guárdalo o ciérralo allá y vuelve a pedirlo",
                        "{} changed on disk since I read it (is it open in another program?): I will not overwrite it. Save or close it there and ask again",
                        self.ruta.display()
                    )
                );
            }
        }
        let dir = self.ruta.parent().unwrap_or(Path::new("."));
        let nombre = self.ruta.file_name().and_then(|s| s.to_str()).unwrap_or("notebook.ipynb");
        let tmp = dir.join(format!(".{nombre}.gotv-{}", std::process::id()));
        std::fs::write(&tmp, &self.texto)?;
        if let Ok(m) = std::fs::metadata(&self.ruta) {
            let _ = std::fs::set_permissions(&tmp, m.permissions());
        }
        if let Err(e) = std::fs::rename(&tmp, &self.ruta) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// El texto, por si se quiere comparar (pruebas).
    pub fn texto(&self) -> &str {
        &self.texto
    }

    /// El lenguaje del notebook (su `kernelspec` o su `language_info`).
    pub fn lenguaje(&self) -> Option<String> {
        self.v["metadata"]["kernelspec"]["language"]
            .as_str()
            .or_else(|| self.v["metadata"]["language_info"]["name"].as_str())
            .map(str::to_lowercase)
    }

    pub fn kernel_nombre(&self) -> Option<String> {
        self.v["metadata"]["kernelspec"]["name"].as_str().map(String::from)
    }
}

/// Un notebook nuevo y vacío, como lo crea nbformat 4.5 (con su kernel si se sabe).
pub fn vacio(kernel: Option<(&str, &str, &str)>) -> String {
    let mut meta = Map::new();
    if let Some((nombre, mostrar, lengua)) = kernel {
        meta.insert(
            "kernelspec".into(),
            serde_json::json!({"display_name": mostrar, "language": lengua, "name": nombre}),
        );
        meta.insert("language_info".into(), serde_json::json!({"name": lengua}));
    }
    let v = serde_json::json!({"cells": [], "metadata": Value::Object(meta), "nbformat": 4, "nbformat_minor": 5});
    let mut o = String::new();
    escribir(&mut o, &v, " ", 0, " ");
    o.push('\n');
    o
}

/// Un texto como lo guarda nbformat: lista de líneas, cada una con su `\n` salvo la última.
pub fn lineas(s: &str) -> Value {
    let mut v = Vec::new();
    let mut resto = s;
    while let Some(k) = resto.find('\n') {
        v.push(Value::from(&resto[..=k]));
        resto = &resto[k + 1..];
    }
    if !resto.is_empty() {
        v.push(Value::from(resto));
    }
    Value::Array(v)
}

/// Un texto de nbformat (cadena o lista de líneas) como una sola cadena.
pub fn texto_de(x: &Value) -> String {
    match x {
        Value::Array(a) => a.iter().filter_map(|l| l.as_str()).collect(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// Un `id` de celda nuevo, como nbformat (`uuid4().hex[:8]`), distinto de los que ya hay.
fn id_nuevo(celdas: &[Value]) -> String {
    loop {
        let mut b = [0u8; 4];
        aleatorio(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        if !celdas.iter().any(|c| c["id"].as_str() == Some(id.as_str())) {
            return id;
        }
    }
}

/// Bytes al azar del sistema (para ids y claves).
pub fn aleatorio(b: &mut [u8]) {
    use std::io::Read;
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(b)).is_err() {
        let n = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for (i, x) in b.iter_mut().enumerate() {
            *x = (n >> ((i % 16) * 8)) as u8 ^ (std::process::id() as u8).wrapping_mul(i as u8 + 1);
        }
    }
}

/// El valor con las claves ordenadas (como `sort_keys=True` de nbformat).
fn ordenado(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut claves: Vec<&String> = m.keys().collect();
            claves.sort();
            let mut o = Map::new();
            for k in claves {
                o.insert(k.clone(), ordenado(&m[k]));
            }
            Value::Object(o)
        }
        Value::Array(a) => Value::Array(a.iter().map(ordenado).collect()),
        x => x.clone(),
    }
}

/// `json.dumps(v, indent=unidad, sort_keys=True, ensure_ascii=False)` de Python, empezando en el nivel `nivel`
/// (la primera línea no lleva sangría: va donde quedó el cursor).
fn escribir(o: &mut String, v: &Value, unidad: &str, nivel: usize, _u: &str) {
    match v {
        Value::Object(m) if !m.is_empty() => {
            let mut claves: Vec<&String> = m.keys().collect();
            claves.sort();
            o.push_str("{\n");
            for (k, c) in claves.iter().enumerate() {
                o.push_str(&unidad.repeat(nivel + 1));
                o.push_str(&serde_json::to_string(c).unwrap_or_default());
                o.push_str(": ");
                escribir(o, &m[*c], unidad, nivel + 1, unidad);
                if k + 1 < claves.len() {
                    o.push(',');
                }
                o.push('\n');
            }
            o.push_str(&unidad.repeat(nivel));
            o.push('}');
        }
        Value::Array(a) if !a.is_empty() => {
            o.push_str("[\n");
            for (k, x) in a.iter().enumerate() {
                o.push_str(&unidad.repeat(nivel + 1));
                escribir(o, x, unidad, nivel + 1, unidad);
                if k + 1 < a.len() {
                    o.push(',');
                }
                o.push('\n');
            }
            o.push_str(&unidad.repeat(nivel));
            o.push(']');
        }
        x => o.push_str(&serde_json::to_string(x).unwrap_or_default()),
    }
}
