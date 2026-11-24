# Contributing to GOD OF THE VOID

*Español abajo.*

GOTV is open source and grows with whoever uses it. Issues, fixes, new tools,
translations and benchmarks are welcome.

## Before you open a pull request

- **Build and test it**: `cargo test --workspace --release` must stay green.
  [BUILDING.md](BUILDING.md) has the requirements.
- **Rust all the way down.** The core, the engines, the data and the logic are
  Rust. The app's JavaScript only *paints*: no logic, no storage, no data files
  of its own. No Python, shell or other languages in the product (code that GOTV
  *writes for its users*, in any language, is a different thing).
- **Two languages, always in pairs.** Every text a user can see exists in
  English and Spanish: `tr!("español", "english")` in Rust, a pair in the UI's
  dictionaries. The judge `cargo test -p gotv-core --test idioma` fails on any
  text left in one language.
- **Measure what you claim.** If a change makes something faster or more
  accurate, say how you measured it and on which Mac.
- **No model weights.** Models are downloaded by the user, with their own
  license, from Settings → Generators. Never add weights to the repository.
- **One change per pull request**, with a message that says *why*.

## Reporting a bug

Open an issue with: your Mac (chip and memory), macOS version, the GOTV
version (`gotv --version`), what you did, what you expected and what happened.
Security problems go through [SECURITY.md](SECURITY.md), not public issues.

By contributing you agree that your work is published under the Apache
License 2.0, like the rest of the project.

---

## Contribuir (español)

Issues, arreglos, herramientas nuevas, traducciones y mediciones son
bienvenidos. Antes de un pull request: las pruebas en verde
(`cargo test --workspace --release`); todo en Rust (el JavaScript de la app sólo
pinta); cada texto visible en sus dos idiomas, con el juez de idioma en verde;
lo que se afirma, medido; ningún peso de modelo en el repositorio; un cambio por
pull request, con su porqué. Al contribuir aceptas publicar tu trabajo bajo la
licencia Apache 2.0, como el resto del proyecto.
