//! Núcleo puro de EVO Linux Control Suite.
//!
//! Este crate **no depende de nadie** (SAD §1.2) y **no hace I/O**: nada de USB,
//! red, disco ni relojes. Eso lo vuelve 100% testeable de forma determinista y
//! permite ejecutar todo el dominio contra un dispositivo simulado
//! ([`evo_mock`](https://docs.rs/evo-mock)).
//!
//! Contiene:
//!
//! - [`model`]: identidad de los modelos de hardware (`ModelId`).
//! - [`profile`]: [`DeviceProfile`] y la matriz de [`Capabilities`] que consulta
//!   la UI para decidir qué controles mostrar (SAD §1.5).
//! - [`command`]: [`DeviceCommand`] — las intenciones que mutan el hardware.
//! - [`event`]: [`DeviceEvent`] (confirmaciones del adaptador) y [`StateEvent`]
//!   (deltas versionados que se difunden a los clientes).
//! - [`state`]: [`DeviceState`], [`StateSnapshot`] con `rev` monótono y la
//!   aplicación determinista de [`StateOp`].
//! - [`safety`]: [`SafetyPolicy`], la defensa principal frente a comandos que
//!   dañan equipos (SAD §4.3). Vive en el núcleo, **nunca** en el cliente.
//! - [`meter`]: [`MeterFrame`], el canal de alta frecuencia, separado del flujo
//!   de comandos (SAD §1.6, §8.2).
//! - [`error`]: errores tipados de dominio.

pub mod channel;
pub mod command;
pub mod error;
pub mod event;
pub mod meter;
pub mod model;
pub mod profile;
pub mod safety;
pub mod state;

pub use channel::ChannelState;
pub use command::{CoalesceKey, DeviceCommand};
pub use error::{CommandRejection, DeviceError, DomainError, MeterError, RepoError, SafetyError};
pub use event::{ConnectionState, DeviceEvent, StateEvent};
pub use meter::MeterFrame;
pub use model::ModelId;
pub use profile::{Capabilities, DeviceProfile, GainRange};
pub use safety::{SafetyContext, SafetyLimits, SafetyPolicy};
pub use state::{
    DeviceState, MonitorMix, PersistedState, SmartGainPhase, StateOp, StateSnapshot,
    state_ops_from_events,
};

/// Capacidad máxima de canales de entrada de cualquier modelo EVO soportado.
///
/// Es solo una cota de dimensionamiento (para buffers de tamaño fijo sin
/// asignaciones, SAD §8.2), **no** una afirmación sobre un modelo concreto: el
/// número real de canales lo define [`DeviceProfile`].
pub const MAX_CHANNELS: usize = 8;

/// Capacidad máxima de salidas. Misma consideración que [`MAX_CHANNELS`].
pub const MAX_OUTPUTS: usize = 8;
