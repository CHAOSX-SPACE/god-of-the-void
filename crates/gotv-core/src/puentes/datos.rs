//! F5 ML9 · el puente de DATOS: las consultas SQL escritas en el código (en cadenas de Rust, Python, TS/JS,
//! Go o PHP) ↔ el esquema REAL de las bases registradas (F1). Avisa de la tabla que no existe y de la
//! columna que su tabla no tiene: el caso típico es una columna quitada o renombrada en la base.
//!
//! El esquema sale de las fotos que guarda `base_datos` al leerlo o cambiarlo (`.gotv/esquemas/*.txt`,
//! ver `datos::fotografiar`): el puente nunca se conecta a una base en cada edición. Sin fotos, no juzga.
use super::{cada_nodo, linea, literal, Desajuste, Fuente};
use gotv_datos::referencias::{parece_consulta, referencias};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// tabla → columnas (en minúsculas), de una foto de esquema («table usuarios: id INTEGER PK, email TEXT»).
pub(crate) fn leer_esquema(t: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut m = BTreeMap::new();
    for l in t.lines().filter(|l| !l.starts_with(' ')) {
        let Some((cabeza, cols)) = l.split_once(": ") else { continue };
        let Some(nombre) = cabeza.split_whitespace().nth(1) else { continue };
        let nombre = nombre.rsplit('.').next().unwrap_or(nombre).trim_matches(|c| c == '"' || c == '`').to_lowercase();
        let cols: BTreeSet<String> = cols.split(", ").filter_map(|c| c.split_whitespace().next()).map(|c| c.trim_matches(|x| x == '"' || x == '`').to_lowercase()).collect();
        m.insert(nombre, cols);
    }
    m
}

/// Todas las tablas conocidas (de todas las bases fotografiadas) y de qué base es cada una.
fn esquemas(root: &Path) -> BTreeMap<String, (String, BTreeSet<String>)> {
    let mut m = BTreeMap::new();
    for e in std::fs::read_dir(crate::datos::dir_esquemas(root)).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "txt") {
            let base = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            for (t, cols) in leer_esquema(&std::fs::read_to_string(&p).unwrap_or_default()) {
                m.entry(t).or_insert((base.clone(), cols));
            }
        }
    }
    m
}

pub(crate) fn revisar(root: &Path, fuentes: &[Fuente]) -> Vec<Desajuste> {
    let tablas = esquemas(root);
    if tablas.is_empty() {
        return vec![];
    }
    let mut v = Vec::new();
    let mut vistas = BTreeSet::new();
    for f in fuentes {
        let bajo = f.texto.to_ascii_lowercase();
        if !["select ", "insert ", "update ", "delete "].iter().any(|k| bajo.contains(k)) {
            continue;
        }
        let Some(a) = f.arbol() else { continue };
        let t = f.texto.as_str();
        cada_nodo(a.raiz(), |n| {
            let Some(sql) = literal(n, t, false).filter(|s| parece_consulta(s)) else { return };
            let Some(r) = referencias(&sql) else { return };
            let lugar = format!("{}:{}", f.rel, linea(n));
            for tabla in &r.tablas {
                if !tablas.contains_key(tabla) && vistas.insert(format!("t|{tabla}|{}", f.rel)) {
                    v.push(Desajuste {
                        puente: "datos".into(),
                        clave: format!("tabla|{tabla}|{}", f.rel), // juez-idioma: datos (clave interna estable)
                        texto: crate::tr!(
                            "{lugar}: la consulta usa la tabla «{tabla}», que no está en ninguna base registrada ({})",
                            "{lugar}: the query uses table «{tabla}», which is not in any registered database ({})",
                            tablas.values().map(|x| x.0.as_str()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", ")
                        ),
                    });
                }
            }
            for (tabla, col) in &r.columnas {
                let Some((base, cols)) = tablas.get(tabla) else { continue };
                if !cols.contains(col) && vistas.insert(format!("c|{tabla}.{col}|{}", f.rel)) {
                    v.push(Desajuste {
                        puente: "datos".into(),
                        clave: format!("columna|{tabla}.{col}|{}", f.rel), // juez-idioma: datos (clave interna estable)
                        texto: crate::tr!(
                            "{lugar}: la consulta usa «{tabla}.{col}», y la tabla «{tabla}» de la base «{base}» no tiene esa columna",
                            "{lugar}: the query uses «{tabla}.{col}», and table «{tabla}» in database «{base}» has no such column"
                        ),
                    });
                }
            }
        });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_las_fotos_de_cada_motor() {
        let m = leer_esquema("table usuarios: id INTEGER PK, email TEXT NOT NULL, saldo numeric(10,2)\nview activos: id INTEGER\ntable public.pedidos [InnoDB]: id bigint, `total` decimal(10,2)\n  índice x\n");
        assert_eq!(m["usuarios"].iter().cloned().collect::<Vec<_>>(), ["email", "id", "saldo"]);
        assert!(m.contains_key("activos") && m["pedidos"].contains("total"));
    }

    #[test]
    fn consultas_del_codigo_contra_la_foto() {
        let d = crate::CarpetaPrueba::nueva("gotv-puente-datos");
        let f = Fuente { rel: "app/db.py".into(), lenguaje: gotv_sintaxis::Lenguaje::Python, texto: "def leer(c):\n    c.execute(\"SELECT id, email FROM usuarios WHERE id = ?\")\n    c.execute(\"SELECT telefono FROM usuarios\")\n    c.execute(\"SELECT * FROM fantasma\")\n    c.execute(f\"SELECT x FROM {tabla}\")\n".into() };
        assert!(revisar(&d, std::slice::from_ref(&f)).is_empty(), "sin fotos no se juzga");
        std::fs::create_dir_all(crate::datos::dir_esquemas(&d)).unwrap();
        std::fs::write(crate::datos::dir_esquemas(&d).join("app.txt"), "table usuarios: id INTEGER PK, email TEXT\n").unwrap();
        let v = revisar(&d, &[f]);
        let claves: Vec<&str> = v.iter().map(|x| x.clave.as_str()).collect();
        assert_eq!(claves, ["columna|usuarios.telefono|app/db.py", "tabla|fantasma|app/db.py"], "{v:?}");
        assert!(v[0].texto.contains("app/db.py:3") && v[0].texto.contains("«app»"), "{}", v[0].texto);
    }
}
