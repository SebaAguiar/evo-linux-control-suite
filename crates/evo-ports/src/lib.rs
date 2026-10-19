//! Puertos de EVO Linux Control Suite.
//!
//! Un puerto es un **trait** que define lo que el núcleo necesita, sin decir
//! cómo se obtiene. Este crate **solo depende de [`evo_domain`]**: ni USB, ni
//! red, ni disco, ni Tauri (SAD §1.2).
//!
//! Definirlos desde el día 1 —aunque solo exista una implementación— es lo que
//! permite cambiar USB real por [`evo_mock`](https://docs.rs/evo-mock) en tests
//! y CI, y soportar modelos nuevos sin tocar el núcleo.

pub mod device;
pub mod meter;
pub mod persist;
pub mod registry;
pub mod sink;
pub mod testkit;

pub use device::{EvoDevice, HwEventCallback, ReopenFn};
pub use meter::MeterSource;
pub use persist::StateRepo;
pub use registry::{DeviceId, DriverFactory, DriverRegistry};
pub use sink::EventSink;

