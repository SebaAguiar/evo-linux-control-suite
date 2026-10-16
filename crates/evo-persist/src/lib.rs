//! Persistencia en disco de EVO Linux Control Suite.
//!
//! Implementa el puerto [`evo_ports::StateRepo`] sobre el documento TOML de la
//! suite en `$XDG_CONFIG_HOME/evo-suite/config.toml` (SAD §3.1, §8.5). Es un
//! **adaptador de salida**: depende de los puertos, nunca al revés.
//!
//! # Qué persiste (y qué no)
//!
//! Guarda el documento completo tal como lo define [`evo_domain::PersistedState`]:
//! los ajustes de la aplicación (`[server]`) y el **estado de dispositivo**, que
//! es el último estado comandado y lo que permite reaplicar ganancia y mute tras
//! un *hot-plug*. La preferencia de tema (`[ui]`) y los presets del SAD §8.5
//! llegarán con los componentes que los consumen (la UI y la biblioteca de
//! presets); no se inventan aquí campos que nadie lee.
//!
//! # Garantías
//!
//! - Escritura **atómica** (temporal + `fsync` + `rename` + `fsync` del directorio).
//! - Copia `.bak` de la versión anterior en cada guardado.
//! - Ante corrupción se recupera del `.bak`; si tampoco sirve, se degrada a "sin
//!   estado", que es un sistema controlable, en lugar de impedir el arranque.

pub mod file_repo;
pub mod xdg;

pub use file_repo::FileStateRepo;
