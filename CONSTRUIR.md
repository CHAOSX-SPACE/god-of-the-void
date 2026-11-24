# Construir GOD OF THE VOID desde el código

*[English](BUILDING.md)*

Para **usar** GOTV no necesitas nada de esto: la app ya armada está en
[Releases](https://github.com/CHAOSX-SPACE/god-of-the-void/releases). Esta página es para
quien quiera armarla con sus propias manos, cambiarla o comprobar que la app que
bajó es este código.

## Qué necesitas

| | | cómo comprobarlo |
|---|---|---|
| Una Mac con **Apple Silicon** (M1 o más nueva) | MLX corre en la GPU de Apple; las Mac con Intel no sirven | `uname -m` → `arm64` |
| **macOS 14** o más nuevo | el mínimo que admite MLX | `sw_vers -productVersion` |
| **Xcode** (la app completa, no sólo las herramientas de línea de órdenes) | compila los kernels Metal y las partes en Objective-C/C++ | `xcodebuild -version` |
| La **cadena Metal** de Xcode | desde Xcode 26 se baja aparte | `xcrun -f metal` |
| **CMake** | compila MLX | `cmake --version` |
| **Rust** estable (armado y probado con 1.99) | todo lo demás | `cargo --version` |

Si `xcrun -f metal` falla, instala la cadena Metal:

```sh
xcodebuild -downloadComponent MetalToolchain
```

CMake, lo más fácil con [Homebrew](https://brew.sh): `brew install cmake`.
Rust, con [rustup](https://rustup.rs).

## Armar la app — una sola orden

```sh
git clone https://github.com/CHAOSX-SPACE/god-of-the-void
cd gotv
cargo run --release -p gotv-empaquetar
```

Queda **`dist/GOD OF THE VOID.app`**. Para dejarla en otra carpeta:
`cargo run --release -p gotv-empaquetar -- <carpeta>`.

El empaquetador (`crates/gotv-empaquetar`, Rust puro y sin dependencias) hace,
en orden:

1. compila en release la app (`gotv-gui`), la terminal (`gotv`), el proceso de
   DuckDB (`gotv-duckdb`) y dos ayudantes de terceros: el del control del
   escritorio de `agent-desktop` y el navegador `agent-browser`;
2. **se niega a seguir** si MLX no quedó compilado para macOS 14, o si sus
   kernels Metal (`mlx.metallib`) salieron para un macOS más nuevo: una app así
   sólo correría en tu propia Mac;
3. arma el paquete: binarios, kernels Metal, la extensión de Chrome, las
   licencias de terceros (`Contents/Resources/licencias`), el ícono y un
   `Info.plist` en inglés con su `es.lproj` en español;
4. lo firma.

**La primera compilación es larga**: baja y compila MLX y DuckDB, dos bases de
código C++ grandes. Las siguientes sólo rehacen lo que cambió.

### La firma

macOS ata los permisos (carpeta Documentos, micrófono, cámara…) a la firma. El
empaquetador firma con, en este orden:

- la identidad de `GOTV_FIRMA`, si la pones:
  `GOTV_FIRMA="Apple Development: …" cargo run --release -p gotv-empaquetar`;
- si no, la primera identidad **Apple Development** de tu llavero (cualquier
  Apple ID gratuito te da una en Xcode → Ajustes → Cuentas);
- si no, **ad hoc**. La app corre, pero macOS ve cada compilación como otra app
  y vuelve a pedir sus permisos cada vez.

Una app que armas y abres en tu propia Mac no queda en cuarentena, así que
Gatekeeper no la bloquea.

**¿Vas a compartir tu compilación?** Usa `GOTV_FIRMA=- cargo run --release -p gotv-empaquetar`.
Una identidad de desarrollo graba en la firma el nombre y el correo de su dueño
(`codesign -dvv` se los muestra a cualquiera); `-` firma ad hoc, sin nadie dentro.
Así se arman las versiones oficiales de GOTV, sin notarizar: GOTV es un proyecto
libre de la comunidad y no le paga a Apple para que lo dejen abrir.

## Desarrollar

```sh
cargo build                          # todos los crates, en debug
cargo run -p gotv-cli -- "arregla el bug de X"   # el agente en la terminal, sobre la carpeta actual
cargo run -p gotv-tui                # la interfaz de terminal
cargo test --workspace --release     # todas las pruebas
cargo test -p gotv-core --test idioma   # el juez de idioma: ningún texto visible en un solo idioma
```

La interfaz de la app (`crates/gotv-gui/ui`) se abre en cualquier navegador con
un núcleo simulado, sin compilar la app:

```sh
cargo run --release -p gotv-gui --example servir_ui -- 8799 crates/gotv-gui/ui
# y abre http://localhost:8799
```

Las pruebas que necesitan un modelo bajado, un equipo o la red llevan
`#[ignore]` y no corren por omisión.

## Modelos

Ningún modelo viaja en el repositorio ni en la app. La primera vez que usas una
función que lo necesita (chat, voz, imágenes, video, música…), GOTV te lleva a
**Configuración → Generadores**, te muestra la licencia del modelo para leerla y
aceptarla, y lo baja de quien lo publica. Los modelos viven en `~/.cache/gotv`.

## Cuando algo falla

| Lo que ves | Por qué | Qué hacer |
|---|---|---|
| `MLX no quedó compilado para macOS 14` | la compilación no leyó `.cargo/config.toml` | compila desde la raíz del repositorio; no cambies `MACOSX_DEPLOYMENT_TARGET` |
| `no encontré mlx.metallib` | falta la cadena Metal | `xcodebuild -downloadComponent MetalToolchain` y vuelve a compilar |
| `cmake: not found` | — | `brew install cmake` |
| la app pide permisos en cada compilación | firma ad hoc | firma con una identidad Apple Development (arriba) |

Si sigue fallando, abre un issue con la salida de la orden:
[github.com/CHAOSX-SPACE/god-of-the-void/issues](https://github.com/CHAOSX-SPACE/god-of-the-void/issues).
