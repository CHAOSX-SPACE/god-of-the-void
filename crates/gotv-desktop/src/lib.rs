//! Control nativo del Mac dentro de GOD OF THE VOID.
//!
//! El motor es **agent-desktop** de Vercel Labs (Apache-2.0; ver
//! `terceros/agent-desktop/NOTICE.md`): lee el árbol de accesibilidad de cada app
//! (API AX de macOS) y actúa sobre sus elementos por *ref*, sin capturas ni
//! coordenadas y sin mover el ratón. Corre DENTRO del ejecutable de GOTV: cada
//! comando en un proceso hijo de GOTV (bandera [`FLAG`]), como el modo MCP de
//! agent-desktop, para poder cancelarlo y aislar un cuelgue de una app.

/// Bandera oculta: el propio binario de GOTV se relanza para ejecutar UN comando.
pub const FLAG: &str = "__gotv-escritorio";

/// Llamar al PRINCIPIO de cada `main` de GOTV. Si este proceso es un hijo de
/// control del Mac o un auxiliar de agent-desktop (cursor visible, asistentes de
/// permisos), hace su trabajo y TERMINA el proceso; si no, vuelve sin hacer nada.
pub fn run_if_requested() {
    #[cfg(target_os = "macos")]
    {
        if let Some(code) = agent_desktop::gotv_helper_if_requested() {
            std::process::exit(code);
        }
        let mut args = std::env::args().skip(1);
        if args.next().as_deref() == Some(FLAG) {
            std::process::exit(agent_desktop::gotv_main(args.collect()));
        }
    }
}
