//! `gotv-duckdb` · DuckDB en un proceso APARTE (decisión de arquitectura, 2026-11-15).
//!
//! Por qué aparte y no dentro de la app: DuckDB es C++ enlazado de 62 MB que se repetía en CADA binario
//! (`gotv` 80 → 143 MB, `gotv-gui` 95 → 157 MB); su compilación de 11 min se repetía por cada combinación
//! de características; un fallo de su C++ tumbaba la app entera; y su memoria (80 % de la RAM por omisión)
//! se quedaba en el proceso de la app. Aquí vive UNA vez: la app lo lanza sólo cuando una acción de
//! `base_datos` toca DuckDB, le habla por líneas JSON (stdin/stdout, [`gotv_datos::duckdb::Pedido`]) y
//! el proceso muere al soltarse la conexión: la RAM vuelve entera y la app no crece.
//!
//! Protocolo: la PRIMERA línea es `abrir`; después, un pedido por línea y una respuesta por línea. Una
//! escritura va en dos tiempos: `ejecutar` responde lo que HARÍA (dentro de la transacción) y espera
//! `decidir` (`COMMIT` o `ROLLBACK`).
mod motor;

use gotv_datos::duckdb::{Pedido, Respuesta};
use std::io::{BufRead, Write};

fn responder(r: &Respuesta) -> bool {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, r).is_ok() && out.write_all(b"\n").is_ok() && out.flush().is_ok()
}

fn error(e: impl std::fmt::Display) -> Respuesta {
    Respuesta::Error(e.to_string())
}

fn main() {
    let entrada = std::io::stdin();
    let mut lineas = entrada.lock().lines();
    let leer = |l: Option<std::io::Result<String>>| -> Option<Pedido> { serde_json::from_str(&l?.ok()?).ok() };
    // 1 · abrir (idioma, proyecto, archivo, modo)
    let (root, mut c) = match leer(lineas.next()) {
        Some(Pedido::Abrir { root, archivo, escribir, en }) => {
            gotv_engine::i18n::set(if en { gotv_engine::i18n::Lang::En } else { gotv_engine::i18n::Lang::Es });
            match motor::abrir(&root, archivo.as_deref(), escribir) {
                Ok(c) => {
                    let v: String = c.query_row("SELECT version()", [], |r| r.get(0)).unwrap_or_default();
                    if !responder(&Respuesta::Texto(v)) {
                        return;
                    }
                    (root, c)
                }
                Err(e) => {
                    responder(&error(e));
                    return;
                }
            }
        }
        _ => {
            responder(&error(gotv_engine::i18n::t("la primera línea debe ser «abrir»", "the first line must be «abrir»")));
            return;
        }
    };
    // 2 · un pedido por línea, hasta que la app suelte la conexión (EOF)
    while let Some(p) = leer(lineas.next()) {
        let r = match p {
            Pedido::Consultar { sql, limites } => motor::consultar(&c, &root, &sql, &limites).map(Respuesta::Tabla).unwrap_or_else(error),
            Pedido::Esquema { tabla } => motor::esquema(&c, tabla.as_deref()).map(Respuesta::Texto).unwrap_or_else(error),
            Pedido::Columnas { archivo } => motor::columnas_de_archivo(&c, &root, &archivo).map(Respuesta::Texto).unwrap_or_else(error),
            Pedido::Explicar { sql, analizar } => motor::explicar(&c, &root, &sql, analizar).map(Respuesta::Texto).unwrap_or_else(error),
            Pedido::Ejecutar { sql } => {
                // dentro de la transacción: se cuenta, se pregunta a la app y se espera su decisión
                let mut cortado = false;
                let r = motor::ejecutar(&mut c, &root, &sql, |e| {
                    if !responder(&Respuesta::Escritura(e.clone())) {
                        cortado = true;
                        return false;
                    }
                    match leer(lineas.next()) {
                        Some(Pedido::Decidir { confirmar }) => confirmar,
                        // la app se fue o mandó otra cosa: ROLLBACK
                        _ => {
                            cortado = true;
                            false
                        }
                    }
                });
                if cortado {
                    return;
                }
                r.map(Respuesta::Escritura).unwrap_or_else(error)
            }
            Pedido::Abrir { .. } | Pedido::Decidir { .. } => error(gotv_engine::i18n::t("pedido fuera de orden", "out-of-order request")),
        };
        if !responder(&r) {
            return;
        }
    }
}
