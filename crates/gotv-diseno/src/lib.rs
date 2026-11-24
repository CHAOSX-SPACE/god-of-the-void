//! GOD OF THE VOID · Ola D — diseño profesional con IAs pequeñas.
//!
//! Ley de la ola: **el modelo decide, el programa construye**. El modelo da intenciones
//! cortas («marca morada, sobria»); aquí se calcula lo que un modelo pequeño hace mal a
//! mano: color con contraste garantizado, escalas, ritmo y movimiento.
//!
//! - [`api`]: la API REST de una entidad en Hono, FastAPI o Axum, con validación, config y pruebas (D8f).
//! - [`app`]: esqueleto Vite + React + TS con la biblioteca, el tema y un CRUD probado (D5).
//! - [`color`]: OKLCH ⇄ sRGB, recorte a la gama y contraste WCAG medido sobre el hex final.
//! - [`tema`]: de un [`tema::Pedido`] a un [`tema::Tema`] completo (claro y oscuro) y su CSS.
//! - [`recursos`]: fuentes OFL e iconos ISC dentro del binario (sin CDN).
//! - [`isometrico`]: escena descrita → SVG isométrico con orden de pintado correcto (D6).
//! - [`pagina`]: ficha corta → página profesional (8 direcciones de estilo, componentes, ilustraciones).

pub mod api;
pub mod app;
pub mod color;
pub mod isometrico;
pub mod pagina;
pub mod recursos;
pub mod tema;
