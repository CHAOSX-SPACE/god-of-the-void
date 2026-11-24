//! F5 ML9 · lo que una consulta SQL NOMBRA: tablas y columnas (resueltas a su tabla cuando se puede). Para
//! el puente de datos de gotv-core: las consultas escritas en el código ↔ el esquema real de la base.
//!
//! Prefiere callar a inventar: una columna sin tabla delante sólo se atribuye si la consulta entera tiene
//! UNA sola tabla; los identificadores entre comillas dobles no se juzgan (en MySQL son textos); los
//! nombres de un `WITH` y los alias de la proyección (`AS total`) no son tablas ni columnas de la base; una
//! función de tabla (`generate_series(…)`) no es una tabla.
use sqlparser::ast::{AssignmentTarget, Expr, ObjectName, Query, Select, SelectItem, Statement, TableFactor, TableObject, Visit, Visitor};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

/// Lo que nombra una consulta (todo en minúsculas).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Referencias {
    pub tablas: BTreeSet<String>,
    /// (tabla, columna)
    pub columnas: BTreeSet<(String, String)>,
}

fn ultimo(n: &ObjectName) -> Option<String> {
    n.0.last().and_then(|p| p.as_ident()).map(|i| i.value.to_lowercase())
}

#[derive(Default)]
struct Colector {
    tablas: Vec<String>,
    /// alias (o el nombre mismo) → tabla
    alias: BTreeMap<String, String>,
    calificadas: Vec<(String, String)>,
    sueltas: Vec<String>,
    ctes: BTreeSet<String>,
    alias_select: BTreeSet<String>,
    /// fuentes que no son una tabla con nombre (subconsultas, funciones de tabla)
    otras: usize,
}

impl Visitor for Colector {
    type Break = ();

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if let Some(w) = &q.with {
            for c in &w.cte_tables {
                self.ctes.insert(c.alias.name.value.to_lowercase());
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, s: &Select) -> ControlFlow<()> {
        for it in &s.projection {
            if let SelectItem::ExprWithAlias { alias, .. } = it {
                self.alias_select.insert(alias.value.to_lowercase());
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, tf: &TableFactor) -> ControlFlow<()> {
        match tf {
            TableFactor::Table { name, alias, args: None, .. } => {
                if let Some(t) = ultimo(name) {
                    if let Some(a) = alias {
                        self.alias.insert(a.name.value.to_lowercase(), t.clone());
                    }
                    self.alias.insert(t.clone(), t.clone());
                    self.tablas.push(t);
                }
            }
            _ => self.otras += 1,
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_statement(&mut self, st: &Statement) -> ControlFlow<()> {
        match st {
            Statement::Insert(i) => {
                if let TableObject::TableName(n) = &i.table {
                    if let Some(t) = ultimo(n) {
                        for c in &i.columns {
                            if let Some(col) = ultimo(c) {
                                self.calificadas.push((t.clone(), col));
                            }
                        }
                        self.alias.insert(t.clone(), t.clone());
                        self.tablas.push(t);
                    }
                }
            }
            Statement::Update(u) => {
                if let TableFactor::Table { name, .. } = &u.table.relation {
                    if let Some(t) = ultimo(name) {
                        for a in &u.assignments {
                            if let AssignmentTarget::ColumnName(c) = &a.target {
                                if let Some(col) = ultimo(c) {
                                    self.calificadas.push((t.clone(), col));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        match e {
            Expr::Identifier(i) if i.quote_style.is_none() => self.sueltas.push(i.value.to_lowercase()),
            Expr::CompoundIdentifier(v) if v.len() >= 2 && v.iter().all(|i| i.quote_style.is_none()) => {
                self.calificadas.push((v[v.len() - 2].value.to_lowercase(), v[v.len() - 1].value.to_lowercase()));
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

/// Lo que nombra `sql`; `None` si no se entiende como SQL.
pub fn referencias(sql: &str) -> Option<Referencias> {
    let st = Parser::parse_sql(&GenericDialect {}, sql).ok()?;
    let mut c = Colector::default();
    let _ = st.visit(&mut c);
    let mut r = Referencias::default();
    for t in &c.tablas {
        if !c.ctes.contains(t) {
            r.tablas.insert(t.clone());
        }
    }
    for (q, col) in &c.calificadas {
        if let Some(t) = c.alias.get(q).filter(|t| !c.ctes.contains(*t)) {
            if col != "*" {
                r.columnas.insert((t.clone(), col.clone()));
            }
        }
    }
    // columnas sueltas: sólo con UNA tabla en toda la consulta
    let unicas: BTreeSet<&String> = c.tablas.iter().collect();
    if unicas.len() == 1 && c.otras == 0 {
        let t = (*unicas.iter().next().unwrap()).clone();
        if !c.ctes.contains(&t) {
            for s in &c.sueltas {
                if !c.alias_select.contains(s) && !c.alias.contains_key(s) {
                    r.columnas.insert((t.clone(), s.clone()));
                }
            }
        }
    }
    Some(r)
}

/// ¿Parece este texto una consulta? (para no pasar cualquier cadena por el analizador)
pub fn parece_consulta(t: &str) -> bool {
    let p = t.trim_start().get(..7).unwrap_or("").to_ascii_lowercase();
    ["select ", "insert ", "update ", "delete ", "with "].iter().any(|k| p.starts_with(k)) && t.trim().len() > 12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(r: &Referencias) -> Vec<String> {
        r.columnas.iter().map(|(t, c)| format!("{t}.{c}")).collect()
    }

    #[test]
    fn tablas_alias_y_columnas() {
        let r = referencias("SELECT u.email, p.total AS t FROM usuarios u JOIN pedidos p ON p.usuario_id = u.id WHERE p.total > ? ORDER BY t").unwrap();
        assert_eq!(r.tablas.iter().cloned().collect::<Vec<_>>(), ["pedidos", "usuarios"]);
        assert_eq!(cols(&r), ["pedidos.total", "pedidos.usuario_id", "usuarios.email", "usuarios.id"]);
        // una sola tabla: las sueltas son suyas (menos el alias de la proyección)
        let r = referencias("SELECT nombre, count(*) AS n FROM clientes WHERE activo = 1 GROUP BY nombre ORDER BY n").unwrap();
        assert_eq!(cols(&r), ["clientes.activo", "clientes.nombre"]);
        // varias tablas: una suelta no se atribuye
        let r = referencias("SELECT nombre FROM a JOIN b ON a.id = b.a_id").unwrap();
        assert_eq!(cols(&r), ["a.id", "b.a_id"]);
    }

    #[test]
    fn escrituras_ctes_y_lo_que_no_se_juzga() {
        let r = referencias("INSERT INTO notas (titulo, cuerpo) VALUES ($1, $2)").unwrap();
        assert_eq!(cols(&r), ["notas.cuerpo", "notas.titulo"]);
        let r = referencias("UPDATE notas SET titulo = :t WHERE id = :id").unwrap();
        assert_eq!(cols(&r), ["notas.id", "notas.titulo"]);
        let r = referencias("WITH recientes AS (SELECT id FROM notas) SELECT id FROM recientes").unwrap();
        assert_eq!(r.tablas.iter().cloned().collect::<Vec<_>>(), ["notas"], "un CTE no es una tabla");
        let r = referencias("SELECT * FROM generate_series(1, 3)").unwrap();
        assert!(r.tablas.is_empty());
        let r = referencias("SELECT \"texto\" FROM t").unwrap();
        assert!(r.columnas.is_empty(), "entre comillas dobles no se juzga");
        assert!(referencias("esto no es sql").is_none());
        assert!(parece_consulta("SELECT id FROM t") && !parece_consulta("Selecciona un archivo") && !parece_consulta("select"));
    }
}
