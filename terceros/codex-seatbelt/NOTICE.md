# Políticas de Seatbelt de Codex dentro de GOD OF THE VOID

Estos tres archivos `.sbpl` vienen de **Codex** de OpenAI
(https://github.com/openai/codex, `codex-rs/sandboxing/src/`), commit `e54a571`
(2026-11-09), bajo la **Apache License 2.0** (ver `LICENSE`, conservada sin cambios).
Copyright de Codex: OpenAI y sus colaboradores. A su vez, Codex los basó en la
política de sandbox de Chromium (citada dentro de cada archivo).

## Uso en GOTV (C2, caja de arena de macOS)
`crates/gotv-core/src/sandbox.rs` los incluye con `include_str!` y les añade, al
armar cada comando: lectura libre, escritura sólo en el proyecto y en las cachés de
herramientas, subrutas protegidas (`.gotv`, `.git/hooks`, `.git/config`) y red sólo a
`localhost` (lo de fuera pasa por el proxy local de GOTV, que decide por dominio).

## Cambios
Ninguno: los tres archivos están tal cual. Para comparar: `git diff --no-index`
contra el commit indicado.
