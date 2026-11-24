# Building GOD OF THE VOID from source

*[Español](CONSTRUIR.md)*

You do not need any of this to **use** GOTV: the ready-made app is on the
[Releases](https://github.com/CHAOSX-SPACE/god-of-the-void/releases) page. This page is for
those who want to build it themselves, change it, or check that the app they
downloaded is this code.

## What you need

| | | check it with |
|---|---|---|
| A Mac with **Apple Silicon** (M1 or newer) | MLX runs on the Apple GPU; Intel Macs are not supported | `uname -m` → `arm64` |
| **macOS 14** or newer | the minimum MLX supports | `sw_vers -productVersion` |
| **Xcode** (the full app, not only the command-line tools) | compiles the Metal kernels and the Objective-C/C++ parts | `xcodebuild -version` |
| Xcode's **Metal toolchain** | Xcode 26 and newer download it apart | `xcrun -f metal` |
| **CMake** | builds MLX | `cmake --version` |
| **Rust**, stable (built and tested with 1.99) | everything else | `cargo --version` |

If `xcrun -f metal` fails, install the Metal toolchain:

```sh
xcodebuild -downloadComponent MetalToolchain
```

CMake through [Homebrew](https://brew.sh) is the easy way: `brew install cmake`.
Rust through [rustup](https://rustup.rs).

## Build the app — one command

```sh
git clone https://github.com/CHAOSX-SPACE/god-of-the-void
cd gotv
cargo run --release -p gotv-empaquetar
```

That leaves **`dist/GOD OF THE VOID.app`**. To put it somewhere else:
`cargo run --release -p gotv-empaquetar -- <folder>`.

The packer (`crates/gotv-empaquetar`, plain Rust with no dependencies) does,
in order:

1. builds in release the app (`gotv-gui`), the CLI (`gotv`), the DuckDB process
   (`gotv-duckdb`) and two vendored helpers: the desktop-control helper of
   `agent-desktop` and the `agent-browser` browser driver;
2. **refuses to go on** if MLX was not compiled for macOS 14, or if its Metal
   kernels (`mlx.metallib`) were built for a newer macOS: such an app would only
   run on your own Mac;
3. assembles the bundle: binaries, Metal kernels, the Chrome extension, the
   third-party licenses (`Contents/Resources/licencias`), the icon and an
   `Info.plist` in English with a Spanish `es.lproj`;
4. signs it.

**The first build is long**: it downloads and compiles MLX and DuckDB, which
are large C++ code bases. The later ones only recompile what changed.

### Signing

macOS ties permissions (Documents folder, microphone, camera…) to the
signature. The packer signs with, in this order:

- the identity in `GOTV_FIRMA`, if you set it:
  `GOTV_FIRMA="Apple Development: …" cargo run --release -p gotv-empaquetar`;
- otherwise the first **Apple Development** identity in your keychain (any free
  Apple ID gives you one through Xcode → Settings → Accounts);
- otherwise **ad hoc**. The app runs, but macOS sees every new build as a
  different app and asks for its permissions again each time.

An app you build and open on your own Mac is not quarantined, so Gatekeeper
does not block it.

**Sharing a build with others?** Use `GOTV_FIRMA=- cargo run --release -p gotv-empaquetar`.
A development identity writes its owner's name and email into the signature
(`codesign -dvv` shows them to anyone); `-` signs ad hoc, with no one inside.
The official GOTV releases are built this way and are not notarized: GOTV is a
free community project and does not pay Apple to be opened.

## Develop

```sh
cargo build                          # every crate, debug
cargo run -p gotv-cli -- "fix the bug in X"     # the agent in the terminal, on the current folder
cargo run -p gotv-tui                # the terminal interface
cargo test --workspace --release     # the whole test suite
cargo test -p gotv-core --test idioma   # the language judge: no user-facing text in one language only
```

The app's interface (`crates/gotv-gui/ui`) can be opened in any browser with
a simulated core, without building the app:

```sh
cargo run --release -p gotv-gui --example servir_ui -- 8799 crates/gotv-gui/ui
# then open http://localhost:8799
```

Tests that need a downloaded model, a device or the network are marked
`#[ignore]` and do not run by default.

## Models

No model ships in the repository or in the app. The first time you use a
function that needs one (chat, voice, images, video, music…), GOTV takes you to
**Settings → Generators**, shows you the model's own license to read and
accept, and downloads it from its publisher. Models live in `~/.cache/gotv`.

## When something fails

| What you see | Why | What to do |
|---|---|---|
| `MLX was not built for macOS 14` | the build ignored `.cargo/config.toml` | build from the repository root; do not override `MACOSX_DEPLOYMENT_TARGET` |
| `mlx.metallib not found` | the Metal toolchain is missing | `xcodebuild -downloadComponent MetalToolchain`, then build again |
| `cmake: not found` | — | `brew install cmake` |
| the app asks for permissions on every build | ad hoc signature | sign with an Apple Development identity (above) |

If it still fails, open an issue with the output of the command:
[github.com/CHAOSX-SPACE/god-of-the-void/issues](https://github.com/CHAOSX-SPACE/god-of-the-void/issues).
