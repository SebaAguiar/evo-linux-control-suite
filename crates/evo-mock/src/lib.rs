//! Dispositivo simulado y adaptadores en memoria.
//!
//! Este crate es la pieza que permite desarrollar y testear **sin hardware**
//! (SAD §1.2, Fase 2): implementa los mismos puertos que el adaptador USB real,
//! así que el núcleo no distingue uno de otro.
//!
//! Incluye también inyección de fallos, para poder probar los caminos de error
//! (desconexión a mitad de un comando, transferencia fallida) que son difíciles de
//! provocar con la unidad física.

pub mod device;
pub mod factory;
pub mod meter;
pub mod repo;

pub use device::{Fault, MockDevice};
pub use factory::MockDriverFactory;
pub use meter::MockMeterSource;
pub use repo::InMemoryStateRepo;
