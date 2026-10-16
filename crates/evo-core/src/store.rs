//! Almacén de estado con escritor único y revisión monótona (SAD §2.4).
//!
//! Es **la única fuente de verdad** del sistema (§8.1): el hardware no notifica
//! cambios físicos, así que aquí vive el *último estado comandado*.
//!
//! Dos invariantes que el resto del sistema da por ciertas:
//!
//! - **Un solo escritor.** Solo el actor de dispositivo muta el almacén. Los
//!   lectores (IPC, Tauri, WebSocket) piden una [`StateSnapshot`] o se suscriben
//!   a deltas; nunca escriben.
//! - **`rev` es monótona y solo avanza cuando algo cambia de verdad.** Un
//!   comando idempotente (`mute` sobre un canal ya silenciado) no genera un
//!   delta: si lo hiciera, cada cliente tendría que reprocesar ruido y un
//!   reconectado no podría distinguir un cambio real.

use evo_domain::{DeviceProfile, DeviceState, DomainError, StateEvent, StateOp, StateSnapshot};

/// Estado canónico versionado.
#[derive(Debug)]
pub struct StateStore {
    rev: u64,
    profile: DeviceProfile,
    state: DeviceState,
}

impl StateStore {
    /// Crea el almacén con una revisión inicial de 0.
    pub fn new(profile: DeviceProfile, state: DeviceState) -> Self {
        Self {
            rev: 0,
            profile,
            state,
        }
    }

    /// Crea el almacén para el estado inicial de un perfil.
    pub fn initial(profile: DeviceProfile) -> Self {
        let state = DeviceState::initial(&profile);
        Self::new(profile, state)
    }

    pub const fn rev(&self) -> u64 {
        self.rev
    }

    pub fn profile(&self) -> &DeviceProfile {
        &self.profile
    }

    pub fn state(&self) -> &DeviceState {
        &self.state
    }

    /// Instantánea completa y consistente para un cliente que acaba de entrar.
    pub fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            rev: self.rev,
            profile: self.profile.clone(),
            state: self.state.clone(),
        }
    }

    /// Aplica un lote de deltas y devuelve el evento difundible.
    ///
    /// Devuelve `Ok(None)` si el lote no cambia nada: en ese caso **no** se
    /// consume una revisión, porque no hay nada que resincronizar.
    ///
    /// Es atómico: si un delta es inválido no se aplica ninguno
    /// ([`DeviceState::apply_all`]), así que el almacén nunca queda a medias.
    pub fn apply(&mut self, ops: &[StateOp]) -> Result<Option<StateEvent>, DomainError> {
        if ops.is_empty() {
            return Ok(None);
        }

        let before = self.state.clone();
        self.state.apply_all(ops)?;
        if self.state == before {
            return Ok(None);
        }

        self.rev += 1;
        Ok(Some(StateEvent {
            rev: self.rev,
            ops: ops.to_vec(),
        }))
    }

    /// Reemplaza el estado por completo, consumiendo una revisión solo si
    /// difiere.
    ///
    /// Sirve para un `read_state` exitoso (SAD §8.6) y para tests que parten de
    /// un estado concreto. El delta que se difunde es el mínimo que explica el
    /// cambio, no un "reemplazo total": el contrato IPC no tiene esa variante y
    /// no se inventa aquí.
    pub fn replace(&mut self, state: DeviceState) -> Option<StateEvent> {
        if self.state == state {
            return None;
        }
        let ops = diff_ops(&self.state, &state);
        self.state = state;
        self.rev += 1;
        Some(StateEvent { rev: self.rev, ops })
    }

    /// Deltas que llevan un estado vacío al estado actual.
    ///
    /// Es el *replay* completo: lo usan la resincronización de un cliente que
    /// detectó un salto de `rev` y la reaplicación tras reconexión.
    pub fn full_delta(&self) -> Vec<StateOp> {
        diff_ops(&DeviceState::initial(&self.profile), &self.state)
    }
}

/// Deltas que transforman `from` en `to`, parámetro a parámetro.
///
/// Vive aquí y no en `evo-domain` porque es una utilidad de difusión, no una
/// regla de negocio: el dominio sabe *aplicar* deltas, no comparar dos estados.
fn diff_ops(from: &DeviceState, to: &DeviceState) -> Vec<StateOp> {
    let mut ops = Vec::new();

    for (i, (a, b)) in from.channels.iter().zip(to.channels.iter()).enumerate() {
        let ch = i as u8;
        if a.gain_db != b.gain_db {
            ops.push(StateOp::Gain { ch, db: b.gain_db });
        }
        if a.muted != b.muted {
            ops.push(StateOp::Mute { ch, on: b.muted });
        }
        if a.phantom != b.phantom {
            ops.push(StateOp::Phantom { ch, on: b.phantom });
        }
        if a.pad != b.pad {
            ops.push(StateOp::Pad { ch, on: b.pad });
        }
        if a.instrument != b.instrument {
            ops.push(StateOp::Instrument {
                ch,
                on: b.instrument,
            });
        }
    }

    for (i, (a, b)) in from.mixes.iter().zip(to.mixes.iter()).enumerate() {
        if a != b {
            ops.push(StateOp::MonitorMix {
                out: i as u8,
                level_db: b.level_db,
                pan: b.pan,
            });
        }
    }

    if from.loopback != to.loopback {
        ops.push(StateOp::Loopback { on: to.loopback });
    }
    if from.smartgain != to.smartgain {
        ops.push(StateOp::SmartGain {
            phase: to.smartgain,
        });
    }

    ops
}
