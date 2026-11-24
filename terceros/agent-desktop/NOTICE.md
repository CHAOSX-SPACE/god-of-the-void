# agent-desktop dentro de GOD OF THE VOID

Este directorio contiene código de **agent-desktop** de Vercel Labs
(https://github.com/vercel-labs/agent-desktop), commit
`b43ac4f6e892ef608f8851428613c4009f0e84c7` (2026-11-05), bajo la licencia
**Apache License 2.0** (ver `LICENSE`, conservada sin cambios).

Copyright de agent-desktop: sus autores (Vercel, Inc. y colaboradores).

## Qué se incluyó
Sólo lo necesario para macOS: `crates/core`, `crates/macos`, `src` (la capa de
comandos), `skills/` (guías que el código incrusta), `tests/` (soporte de pruebas),
`README.md`, `AGENTS.md`, `CONCEPTS.md`, `.github/workflows/ci.yml` (incrustado por
una prueba) y `Cargo.lock`. Se excluyeron Windows, Linux, la FFI en C, npm y docs.

## Cambios hechos por GOD OF THE VOID (Apache-2.0 §4b)
1. `Cargo.toml` (raíz): el workspace sólo lista `crates/core`, `crates/macos` y `src`.
2. `src/Cargo.toml`: `src` se compila como **biblioteca** (`[lib] path = "main.rs"`)
   en vez de binario; se quitaron las dependencias de Windows/Linux y los `[[test]]`
   que lanzan el binario.
3. `src/main.rs`: se AÑADIERON al final `gotv_helper_if_requested()` y `gotv_main()`
   (con sus auxiliares `gotv_emit` y `gotv_parse_failure`), que ejecutan la MISMA
   tubería del CLI con argumentos dados. El resto del archivo no cambió.
4. `src/main.rs`: `#![allow(dead_code)]` (como biblioteca, el CLI y el MCP no se usan).
5. `crates/macos/src/system/cursor_overlay/spawn.rs`: el cursor visible también se
   lanza cuando el ejecutable es de GOTV (`gotv`, `gotv-gui`, `gotv-tui`), no sólo
   `agent-desktop`.

No se cambió ninguna otra línea. Para comparar: `git diff --no-index` contra el
commit indicado.

## Cambios de GOD OF THE VOID (2026-11-10)
- `crates/macos/build.rs`: el mínimo de macOS del puente Objective-C se puede fijar aparte con
  `AGENT_DESKTOP_MACOS_MIN` (GOTV compila el resto con `MACOSX_DEPLOYMENT_TARGET=14.0` por MLX; el
  puente conserva 10.15, para el que está escrito su código y su `-Werror`). Sin la variable, igual que antes.
