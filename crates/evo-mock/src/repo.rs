//! Implementación en memoria del puerto [`StateRepo`].

use std::sync::Mutex;

use evo_domain::{PersistedState, RepoError};
use evo_ports::StateRepo;

/// Almacén de estado en memoria.
///
/// Sirve para dos cosas: tests del núcleo (sin tocar el disco) y el modo de
/// arranque "sin persistencia", en el que el control funciona igual pero nada
/// sobrevive al cierre (SAD §8.5: un fallo de persistencia no debe impedir
/// controlar el hardware).
#[derive(Default)]
pub struct InMemoryStateRepo {
    slot: Mutex<Option<PersistedState>>,
}

impl InMemoryStateRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Devuelve una copia del estado guardado, si lo hay.
    pub fn peek(&self) -> Option<PersistedState> {
        self.slot.lock().expect("mutex envenenado").clone()
    }
}

impl StateRepo for InMemoryStateRepo {
    fn load(&self) -> Result<Option<PersistedState>, RepoError> {
        Ok(self.slot.lock().expect("mutex envenenado").clone())
    }

    fn save(&self, state: &PersistedState) -> Result<(), RepoError> {
        *self.slot.lock().expect("mutex envenenado") = Some(state.clone());
        Ok(())
    }
}
