//! Eventos: confirmaciones del adaptador y deltas de estado difundidos.

use crate::state::SmartGainPhase;
use serde::{Deserialize, Serialize};

/// Estado de conexión del dispositivo.
///
/// Máquina de estados tipada: `Disconnected → Enumerating → Connected →
/// Degraded` (SAD §1.6). `Degraded` cubre el caso de dispositivo presente pero
/// con el plano de control fallando de forma intermitente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Enumerating,
    Connected,
    Degraded,
}

/// Evento emitido por el adaptador tras aplicar un comando o detectar un cambio.
///
/// Es el vocabulario común de *todos* los orígenes (USB real, mock, y en el
/// futuro un interrupt endpoint): así los clientes ven lo mismo sin importar de
/// dónde vino el cambio (SAD §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum DeviceEvent {
    GainChanged { ch: u8, db: f32 },
    MuteChanged { ch: u8, on: bool },
    PhantomChanged { ch: u8, on: bool },
    PadChanged { ch: u8, on: bool },
    InstrumentChanged { ch: u8, on: bool },
    MonitorMixChanged { out: u8, level_db: f32, pan: f32 },
    LoopbackChanged { on: bool },
    SmartGainChanged { phase: SmartGainPhase },
    ConnectionChanged(ConnectionState),
}

/// Delta versionado de estado, difundido a los clientes.
///
/// `rev` es el contador monótono del `StateStore`. Si un cliente detecta un
/// salto de `rev`, pide resincronización (SAD §2.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateEvent {
    pub rev: u64,
    pub ops: Vec<crate::state::StateOp>,
}
