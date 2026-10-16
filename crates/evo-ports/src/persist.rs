//! Puerto de persistencia.

use evo_domain::{PersistedState, RepoError};

/// Almacén del último estado conocido y de la configuración.
///
/// En v1.0 la implementación es de archivo (TOML/JSON, SAD §8.5); migrar a SQLite
/// en v1.x no debe tocar el núcleo. Es **dyn-compatible** a propósito: el
/// composition root elige la implementación en tiempo de arranque.
///
/// La escritura es responsabilidad de un único proceso (el daemon). La UI nunca
/// escribe el archivo directamente: envía comandos (SAD §8.5).
pub trait StateRepo: Send + Sync {
    /// Carga el estado persistido.
    ///
    /// Devuelve `Ok(None)` si no hay nada guardado todavía — que es distinto de
    /// un error: en el primer arranque no existe documento, y eso no debe
    /// impedir controlar el hardware (SAD §8.5, tolerancia a corrupción).
    fn load(&self) -> Result<Option<PersistedState>, RepoError>;

    /// Guarda el estado de forma atómica (temporal + `fsync` + `rename`),
    /// conservando una copia `.bak` de la versión anterior.
    fn save(&self, state: &PersistedState) -> Result<(), RepoError>;
}
