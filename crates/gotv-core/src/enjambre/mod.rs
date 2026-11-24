//! F9 · Varios agentes de verdad: repartir una tarea grande entre varios subagentes que trabajan A LA VEZ,
//! cada uno con su conversación, sus permisos y su rama, y un verificador que no es el que escribió el código
//! (y puede ir en otro modelo).
//!
//! | Pieza | Dónde | Qué hace |
//! |---|---|---|
//! | Subagente como `Agent` propio | `agent/paralelo.rs` | toma PRESTADO el motor (no lo copia ni lo carga otra vez) y lleva sus permisos, su punto de restauración y su rama |
//! | Planificador | [`planificador`] | nube → en paralelo (con tope); local → por lotes si el motor los tiene ([`gotv_engine::Engine::lote_max`]), si no por turnos |
//! | Ramas | [`ramas`] | lo de cada rama llega al árbol si no choca; lo que choca se reporta y su rama queda |
//! | Tablero | [`tablero`] | notas comunes del equipo, en orden, en `.gotv/chats/<chat>/tablero.gotv` (códec propio) |
//! | Coordinador | [`coordinador`] | la skill incluida `enjambre`: cómo partir el trabajo para que no choque |
//! | Verificador | [`verificador`] | otra conversación (y si se pide otro modelo) que corre las pruebas y NO edita |
//! | Buzón | [`buzon`] | mensajes entre sesiones de GOTV, que cada sesión mira entre pasos |
//! | Herramienta | [`herramienta`] | `enjambre`: anotar/tablero, fusionar, verificar, mensaje, sesiones |
//!
//! Lo que manda F9.0 (medir la generación en lote con el modelo local): con un motor local que no genera en
//! lote (`lote_max() == 1`, hoy el núcleo nativo) los subagentes van por TURNOS, uno detrás de otro, igual
//! de aislados; si la medición confirma la ganancia y el núcleo la implementa, el mismo código los corre por
//! LOTES sin cambiar nada aquí. Los dos resultados posibles de F9.0 caben en este diseño.

pub mod buzon;
pub mod coordinador;
pub mod herramienta;
pub mod planificador;
pub mod ramas;
pub mod tablero;
pub mod verificador;
