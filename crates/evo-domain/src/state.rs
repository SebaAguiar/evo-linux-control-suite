//! Estado canónico, snapshot versionado y aplicación determinista de deltas.

use crate::channel::ChannelState;
use crate::error::DomainError;
use crate::event::DeviceEvent;
use crate::profile::DeviceProfile;
use serde::{Deserialize, Serialize};

/// Fase del ciclo SmartGain (SAD §1.6):
/// `Idle → Listening → Calibrating → Success | Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartGainPhase {
    Idle,
    Listening,
    Calibrating,
    Success,
    Error,
    Cancelled,
}

impl SmartGainPhase {
    /// ¿Es un estado terminal del ciclo?
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            SmartGainPhase::Idle
                | SmartGainPhase::Success
                | SmartGainPhase::Error
                | SmartGainPhase::Cancelled
        )
    }

    /// ¿Hay un ciclo en curso?
    pub const fn is_running(self) -> bool {
        matches!(
            self,
            SmartGainPhase::Listening | SmartGainPhase::Calibrating
        )
    }
}

/// Estado de un bus de monitorización.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MonitorMix {
    pub level_db: f32,
    /// Paneo en `[-1.0, 1.0]`.
    pub pan: f32,
}

impl Default for MonitorMix {
    fn default() -> Self {
        Self {
            level_db: 0.0,
            pan: 0.0,
        }
    }
}

/// Estado canónico del dispositivo.
///
/// Es **la única fuente de verdad** (SAD §8.1): el hardware no notifica cambios
/// físicos, así que lo que aquí vive es el *último estado comandado*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub channels: Vec<ChannelState>,
    pub mixes: Vec<MonitorMix>,
    pub loopback: bool,
    pub smartgain: SmartGainPhase,
}

impl DeviceState {
    /// Estado inicial coherente con un perfil: canales y buses en sus valores
    /// por defecto, phantom apagado, loop-back apagado.
    pub fn initial(profile: &DeviceProfile) -> Self {
        Self {
            channels: vec![ChannelState::default(); profile.inputs as usize],
            mixes: vec![MonitorMix::default(); profile.outputs as usize],
            loopback: false,
            smartgain: SmartGainPhase::Idle,
        }
    }

    /// Aplica un delta, validando índices contra el perfil.
    ///
    /// Es determinista y sin I/O: la lógica de mutación del `StateStore` se
    /// prueba aquí sin necesidad de un dispositivo.
    pub fn apply(&mut self, op: &StateOp) -> Result<(), DomainError> {
        match *op {
            StateOp::Gain { ch, db } => {
                self.channel_mut(ch)?.gain_db = db;
            }
            StateOp::Mute { ch, on } => {
                self.channel_mut(ch)?.muted = on;
            }
            StateOp::Phantom { ch, on } => {
                self.channel_mut(ch)?.phantom = on;
            }
            StateOp::Pad { ch, on } => {
                self.channel_mut(ch)?.pad = on;
            }
            StateOp::Instrument { ch, on } => {
                self.channel_mut(ch)?.instrument = on;
            }
            StateOp::MonitorMix { out, level_db, pan } => {
                let outputs = self.mixes.len() as u8;
                let mix = self
                    .mixes
                    .get_mut(out as usize)
                    .ok_or(DomainError::OutputOutOfRange { out, outputs })?;
                mix.level_db = level_db;
                mix.pan = pan;
            }
            StateOp::Loopback { on } => self.loopback = on,
            StateOp::SmartGain { phase } => self.smartgain = phase,
        }
        Ok(())
    }

    /// Aplica un lote de deltas de forma atómica: si uno falla, no se aplica
    /// ninguno. Evita dejar el estado a medias ante un índice inválido.
    pub fn apply_all(&mut self, ops: &[StateOp]) -> Result<(), DomainError> {
        let mut next = self.clone();
        for op in ops {
            next.apply(op)?;
        }
        *self = next;
        Ok(())
    }

    fn channel_mut(&mut self, ch: u8) -> Result<&mut ChannelState, DomainError> {
        let channels = self.channels.len() as u8;
        self.channels
            .get_mut(ch as usize)
            .ok_or(DomainError::ChannelOutOfRange { ch, channels })
    }
}

/// Delta de estado con ruta estable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "path", rename_all = "snake_case")]
pub enum StateOp {
    Gain { ch: u8, db: f32 },
    Mute { ch: u8, on: bool },
    Phantom { ch: u8, on: bool },
    Pad { ch: u8, on: bool },
    Instrument { ch: u8, on: bool },
    MonitorMix { out: u8, level_db: f32, pan: f32 },
    Loopback { on: bool },
    SmartGain { phase: SmartGainPhase },
}

impl StateOp {
    /// Ruta textual del parámetro, en el formato del contrato IPC
    /// (`"ch/0/gain"`, SAD §4.6).
    pub fn path(&self) -> String {
        match *self {
            StateOp::Gain { ch, .. } => format!("ch/{ch}/gain"),
            StateOp::Mute { ch, .. } => format!("ch/{ch}/mute"),
            StateOp::Phantom { ch, .. } => format!("ch/{ch}/phantom"),
            StateOp::Pad { ch, .. } => format!("ch/{ch}/pad"),
            StateOp::Instrument { ch, .. } => format!("ch/{ch}/instrument"),
            StateOp::MonitorMix { out, .. } => format!("out/{out}/mix"),
            StateOp::Loopback { .. } => "loopback".to_string(),
            StateOp::SmartGain { .. } => "smartgain".to_string(),
        }
    }

    /// Clave de parámetro para *last-write-wins* (SAD §2.4): dos ops con la misma
    /// clave afectan al mismo parámetro y la última gana.
    pub fn param_key(&self) -> String {
        match *self {
            StateOp::MonitorMix { out, .. } => format!("out/{out}/mix"),
            StateOp::Loopback { .. } => "loopback".to_string(),
            StateOp::SmartGain { .. } => "smartgain".to_string(),
            other => other.path(),
        }
    }
}

impl DeviceEvent {
    /// Traduce una confirmación del adaptador al delta de estado equivalente.
    ///
    /// Devuelve `None` para eventos que **no** forman parte del estado canónico
    /// persistido: el estado de conexión es efímero (SAD §2.4).
    pub fn to_state_op(&self) -> Option<StateOp> {
        match *self {
            DeviceEvent::GainChanged { ch, db } => Some(StateOp::Gain { ch, db }),
            DeviceEvent::MuteChanged { ch, on } => Some(StateOp::Mute { ch, on }),
            DeviceEvent::PhantomChanged { ch, on } => Some(StateOp::Phantom { ch, on }),
            DeviceEvent::PadChanged { ch, on } => Some(StateOp::Pad { ch, on }),
            DeviceEvent::InstrumentChanged { ch, on } => Some(StateOp::Instrument { ch, on }),
            DeviceEvent::MonitorMixChanged { out, level_db, pan } => {
                Some(StateOp::MonitorMix { out, level_db, pan })
            }
            DeviceEvent::LoopbackChanged { on } => Some(StateOp::Loopback { on }),
            DeviceEvent::SmartGainChanged { phase } => Some(StateOp::SmartGain { phase }),
            DeviceEvent::ConnectionChanged(_) => None,
        }
    }
}

/// Convierte una lista de eventos de dispositivo en los deltas equivalentes.
pub fn state_ops_from_events(events: &[DeviceEvent]) -> Vec<StateOp> {
    events.iter().filter_map(DeviceEvent::to_state_op).collect()
}

/// Instantánea completa versionada que recibe un cliente al conectarse
/// (SAD §2.4, §4.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub rev: u64,
    pub profile: DeviceProfile,
    pub state: DeviceState,
}

/// Último estado conocido, apto para persistir y reaplicar al reconectar.
///
/// Como el hardware no permite leer su estado (SAD §8.1, §8.6), esta es la
/// memoria que permite reaplicar ganancia y mute tras un hot-plug. El phantom
/// **no** se reaplica automáticamente: requiere confirmación (SAD §8.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    /// Versión del documento, para migraciones futuras (`schema_version = 1`).
    pub schema_version: u32,
    /// Marca temporal (epoch ms) de la última escritura.
    ///
    /// Va **antes** de los campos que son tablas (`profile`, `state`) a
    /// propósito: los serializadores de TOML deben emitir todos los valores
    /// escalares antes de la primera tabla, y un escalar declarado después de una
    /// tabla hace fallar la serialización.
    pub saved_at_ms: u64,
    pub profile: DeviceProfile,
    pub state: DeviceState,
}

/// Versión actual del esquema de [`PersistedState`].
pub const SCHEMA_VERSION: u32 = 1;

impl PersistedState {
    pub fn new(profile: DeviceProfile, state: DeviceState, saved_at_ms: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            saved_at_ms,
            profile,
            state,
        }
    }
}
