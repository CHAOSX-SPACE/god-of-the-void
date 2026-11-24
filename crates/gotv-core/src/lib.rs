//! GOD OF THE VOID · núcleo — agente + herramientas + permisos + frontend + config.

pub mod agent;
// F7 · plan, tareas y recorrido de cada conversación, comentarios por línea, video del navegador y sitios
pub mod artefactos;
pub mod audio;
pub mod auto;
pub mod biblioteca;
pub mod apps;
pub mod excel;
pub mod ooxml;
pub mod word;
pub mod carta;
pub mod ppt;
pub mod pdf;
pub mod pdf_diseno;
pub mod ojos;
pub mod browser;
pub mod canal;
// F8.2 · el panel de cambios (por turno sin git, y con git)
pub mod cambios;
pub mod checkpoint;
pub mod chrome;
pub mod computer;
pub mod conexiones;
pub mod config;
pub mod conversaciones;
pub mod docs;
pub mod entorno;
pub mod entrega;
pub mod estilo;
pub mod exec;
pub mod experto;
pub mod experto_analisis;
pub mod experto_chat;
pub mod experto_dsp;
pub mod experto_export;
pub mod experto_fx;
pub mod experto_ia;
pub mod generadores;
pub mod frontend;
pub mod guardia;
pub mod hardware;
pub mod hooks;
/// F10.1 · dónde dispara cada evento de los hooks dentro del agente
pub mod hooks_eventos;
/// F10.4 · plugins propios (`~/.config/gotv/plugins/<nombre>/`) y de Claude Code, con confianza
pub mod plugins;
pub mod hub;
pub mod images;
pub mod instalado;
pub mod lenguajes;
pub mod paquetes;
/// F20.1 · `package auditar`: los archivos de bloqueo contra OSV
pub mod auditoria;
pub mod perfil;
pub mod pro;
/// Idioma (P8): vive en gotv-engine para que TODAS las capas usen el mismo par.
pub use gotv_engine::i18n;
pub use gotv_engine::tr;
pub mod launch;
pub mod lectura;
pub mod llaves;
pub mod lsp;
pub mod mapa;
pub mod mcp;
/// F13.3 · transportes MCP remotos (HTTP streamable, SSE, WebSocket) como puentes hacia el canal.
pub mod mcp_red;
/// F13.4 · OAuth de los servidores MCP remotos (tokens en el Llavero).
pub mod mcp_oauth;
/// F13.10 · GOTV como servidor MCP por stdio (`gotv mcp serve`).
pub mod mcp_serve;
pub mod mcp_eco;
// F8.6 · buscar en todos los chats (⌘K)
pub mod hallar;
pub mod memory;
// F8.3 · menciones con @ (archivos, chats, reglas, recursos MCP) y citas
pub mod menciones;
pub mod mensajes;
pub mod models;
pub mod netproxy;
pub mod nube;
pub mod paths;
pub mod permissions;
pub mod propios;
pub mod rag;
// F8.4 · varias carpetas por proyecto
pub mod raices;
pub mod salidas;
/// F1 · la herramienta `base_datos`.
pub mod datos;
/// F19 · contenedores y servicios del proyecto (API del motor por su socket, fuera de la caja, con frenos).
pub mod contenedores;
/// F17 · git propio: lecturas con gix, escrituras con el binario y sus frenos
pub mod git;
/// F18 · móvil: simulador de iOS y emulador o teléfono Android (herramienta `dispositivo`)
pub mod movil;
/// F17 · el lado que escribe de `github` (PR, comentarios, CI); nunca fusiona
pub mod github_pr;
/// F3 · vigilar y reaccionar: procesos, comandos, archivos, WebSocket y PR/CI despiertan al agente
pub mod vigilar;
/// F10.2 · `gotv -p` sin nadie a quien preguntar: salida JSON (un objeto o una línea por evento) y códigos
pub mod flujo;
/// F12 · formas de trabajar: `/objetivo`, `/aparte`, `/aprender`, `/entrevista` y reglas por ruta
pub mod formas;
/// F16 · paridad con la app de escritorio: escribir mientras trabaja, agentes a medida, variables guardadas,
/// `/revisar`, seguir un PR, editor y terminal
pub mod escritorio;
/// F16.3 · la capa `Lugar` (local o remoto) bajo las herramientas: proyectos que viven en otra máquina
pub mod lugar;
/// F2 · automatizaciones: `programar` en el chat, tareas programadas y su `LaunchAgent`
pub mod programar;
/// F2.1 · notificaciones del sistema (¿se avisa? es una función pura)
pub mod avisos;
/// F5 ML2 · la compuerta del turno: verificación completa de los lenguajes tocados al cerrar.
pub mod compuerta;
/// F1 paso 6 · túnel SSH hacia una base remota.
pub mod tunel;
pub mod recover;
pub mod pruebas;
/// F15.2 · `depurar`: un depurador por el protocolo DAP (lldb-dap, dlv, debugpy)
pub mod depurar;
/// F4 · `notebook`: editar celdas de un .ipynb y ejecutarlas con el kernel del proyecto (Jupyter por ZeroMQ)
pub mod cuaderno;
/// F22 · equipos conectados: USB, discos, puertos serie, ESP32, Pico, Raspberry, impresoras y cámaras
pub mod equipos;
/// F5 ML9 · puentes entre lenguajes (Tauri, HTTP, entorno): avisa del contrato que deja de casar
pub mod puentes;
pub mod revisor;
pub mod research;
/// F23 · búsqueda con varios buscadores, fichas legales del guardado (F23.L) y fusión con citas
pub mod buscar;
/// F23.3 · el índice propio (FTS5) y su rastreador cortés
pub mod indice;
/// F23.4 · el navegador hablado directo por CDP (Chromium aislado, sin capturas)
pub mod navegador;
/// F23.5 · la herramienta `investigar`: busca, lee, redacta con citas y las comprueba
pub mod investigar;
pub mod sandbox;
pub mod secretos;
pub mod serve;
/// F10.5 · control remoto desde el teléfono: sesiones vivas, eventos por SSE y permisos (rutas `/gotv/`)
pub mod remoto;
/// F10.3 · GOTV para editores por el Agent Client Protocol (`gotv acp`)
pub mod acp;
pub mod session;
pub mod skills;
// F8.5 · charlas fuera de proyecto
pub mod sueltas;
pub mod toolchain;
pub mod tema_doc;
pub mod tools;
pub mod web;
pub mod worktree;
/// F9 · varios agentes de verdad: planificador, ramas, tablero, verificador, buzón entre sesiones
pub mod enjambre;

/// Carpetas temporales de las pruebas: se borran solas al soltarse (ver `gotv_engine::prueba`).
#[cfg(test)]
pub(crate) use gotv_engine::prueba::CarpetaPrueba;
pub use agent::Agent;
pub use checkpoint::Checkpoint;
pub use config::Config;
pub use frontend::{Frontend, StdFrontend, TurnStats};
pub use hardware::Hardware;
pub use models::{discover, fit, Fit, ModelCard};
pub use permissions::{AskAnswer, Decision, Mode, Permissions};
pub use rag::Rag;
