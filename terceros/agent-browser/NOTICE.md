# agent-browser dentro de GOD OF THE VOID

Este directorio contiene código de **agent-browser** de Vercel Labs
(https://github.com/vercel-labs/agent-browser), commit `edc992b`
(2026-11-09), bajo la licencia **Apache License 2.0** (ver `LICENSE`, conservada sin
cambios). Copyright de agent-browser: sus autores (Vercel, Inc. y colaboradores).

## Qué se incluyó
`cli/` (el crate de Rust), `skill-data/` y `skills/` (guías que el CLI sirve en tiempo de
ejecución), `docs/content/docs/configuration.mdx` (lo incrusta una prueba), `README.md` y
`LICENSE`. Se excluyeron el panel web (Next.js), los paquetes npm, docker, evals,
benchmarks y ejemplos.

## Cambios hechos por GOD OF THE VOID (Apache-2.0 §4b)
1. `packages/dashboard/out/index.html`: página mínima en lugar del panel web compilado
   (el crate lo incrusta con `rust-embed`; GOTV no lo usa).
2. `cli/src/native/actions.rs`, `handle_pdf` (2026-11-14): `preferCSSPageSize` vale `true` por
   omisión, para que `pdf` respete el `@page { size }` de la página (los PDF de diseño de GOTV en A4,
   carta u oficio). Una página sin tamaño en su CSS sale en carta, igual que antes.

No se cambió ninguna otra línea. Para comparar: `git diff --no-index` contra el commit
indicado.

## Uso en GOTV (W1)
GOTV compila `cli/` como el ejecutable `agent-browser` y lo empaqueta junto al suyo
(`herramientas/empaquetar.sh`). La herramienta nativa `browser` lo llama con una sesión
por chat (`AGENT_BROWSER_SESSION`), usando el Chrome instalado con un perfil TEMPORAL y
aislado (nunca el del usuario).
