---
name: lenguaje-rust
description: Rust con Cargo, paso a paso — correr UNA prueba, añadir una prueba, dependencias (versiones, features), depurar un fallo y cobertura. Úsala cuando trabajes en un proyecto Rust y dudes del comando exacto.
---

# Rust (Cargo)

## Correr una prueba
- Todas: `cargo test` (en un workspace: `cargo test --workspace`).
- Una por nombre (contiene): `cargo test nombre_de_la_prueba`; exacta: `cargo test nombre -- --exact`.
- De un crate: `cargo test -p crate`; sólo las de integración de un archivo: `cargo test --test archivo`.
- Ver lo que imprime: `cargo test nombre -- --nocapture`.
- Mejor: la herramienta `pruebas` (accion=correr, que=Nombre) da el resultado ya leído, con `archivo:línea`.

## Añadir una prueba
- Unitaria junto al código: `#[cfg(test)] mod tests { use super::*; #[test] fn caso() { assert_eq!(f(2), 4); } }`.
- De integración: `tests/nombre.rs` (sólo ve lo público del crate).
- Asíncrona con tokio: `#[tokio::test] async fn caso() { … }`.

## Dependencias
- Añadir: `cargo add serde --features derive` (nunca editar a ciegas la versión en Cargo.toml).
- Por qué está una: `cargo tree -i nombre`; duplicadas: `cargo tree -d`.
- Una versión que no compila: fija la anterior con `cargo add nombre@=1.2.3` y dilo.

## Depurar
- El primer error del compilador es el que manda: corrige ése y vuelve a compilar.
- Traza de un pánico: `RUST_BACKTRACE=1 cargo test nombre`.
- Valores intermedios: `dbg!(x)` (sale por stderr; quítalo al terminar).

## Cobertura
- `cargo llvm-cov` si está instalado (`cargo llvm-cov --html`); si no, dilo y no lo instales sin permiso.
