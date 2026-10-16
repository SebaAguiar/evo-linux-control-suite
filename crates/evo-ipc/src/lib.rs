//! Contrato de mensajes compartido por la UI de escritorio y el control remoto
//! (SAD §4.6).
//!
//! Un único conjunto de tipos `serde` describe el protocolo, de modo que
//! escritorio y móvil hablen exactamente lo mismo. Se separan dos canales:
//!
//! - **Control (JSON):** [`ClientMessage`] de cliente a servidor y
//!   [`ServerMessage`] de servidor a cliente.
//! - **Medidores (binario):** fotogramas compactos de
//!   [`MeterFrame`](evo_domain::MeterFrame), ~1 byte por canal, que viajan como
//!   mensajes binarios del mismo WebSocket para no competir con el control
//!   (SAD §2.2, §8.2).
//!
//! # Versión del protocolo
//!
//! El primer mensaje del servidor es [`ServerMessage::Hello`], que anuncia
//! [`PROTO_VERSION`]; así un móvil con la página cacheada detecta que habla con
//! un servidor más nuevo y puede recargarse (SAD §4.6).
//!
//! # Generación de tipos TypeScript
//!
//! El SAD (§4.6) prevé generar los tipos TypeScript con `ts-rs` o `specta`. Se
//! **difiere** hasta que exista un consumidor real (los frontends): hoy serían
//! anotaciones de mantenimiento sin nadie que las use. El contrato en Rust es la
//! fuente de verdad y no cambia al añadirlo; queda como evolución de v1.x.

mod message;
mod patch;

pub use message::{ClientMessage, ErrorCode, ServerMessage};
pub use patch::PatchOp;

/// Versión del protocolo de mensajes.
///
/// Se envía en [`ServerMessage::Hello`]. Es independiente de
/// `PersistedState::schema_version`: una versiona el cable, la otra el archivo.
pub const PROTO_VERSION: u16 = 1;
