//! Comandos: las intenciones que mutan el hardware (SAD §1.5).

use serde::{Deserialize, Serialize};

/// Comando dirigido a un dispositivo EVO.
///
/// `SetPhantom` reemplaza al `toggle_phantom` del documento funcional original:
/// ser idempotente elimina los errores por estado desincronizado (SAD §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DeviceCommand {
    /// Ajusta la ganancia de un canal de entrada, en dB.
    SetGain { ch: u8, db: f32 },
    /// Silencia o desilencia un canal de entrada.
    SetMute { ch: u8, on: bool },
    /// Conmuta phantom power (+48 V). Sujeto a `SafetyPolicy`.
    SetPhantom { ch: u8, on: bool },
    /// Ajusta nivel y paneo de un bus de monitorización.
    SetMonitorMix { out: u8, level_db: f32, pan: f32 },
    /// Activa o desactiva loop-back.
    SetLoopback { on: bool },
    /// Dispara el asistente de ganancia automática.
    ///
    /// El nombre en el cable se fija explícitamente a `trigger_smartgain` (y no
    /// al `trigger_smart_gain` que produciría `snake_case`) porque así lo fija el
    /// contrato IPC (SAD §4.6) y así lo devuelve [`DeviceCommand::op_name`]: el
    /// `rename_all` de la variante parte `SmartGain` en dos palabras.
    #[serde(rename = "trigger_smartgain")]
    TriggerSmartGain,
    /// Cancela un SmartGain en curso.
    #[serde(rename = "cancel_smartgain")]
    CancelSmartGain,
    /// Atenuación de entrada (pad), si el modelo la soporta.
    SetPad { ch: u8, on: bool },
    /// Entrada de instrumento (DI), si el modelo la soporta.
    SetInstrument { ch: u8, on: bool },
}

/// Clave de *coalescing* para ráfagas de comandos.
///
/// Arrastrar un slider de ganancia produce cientos de eventos por segundo y cada
/// control transfer cuesta ~1 ms de bus (SAD §2.2). El actor agrupa por clave y
/// conserva solo el último valor (*last-write-wins*) dentro de una ventana de
/// 8–16 ms.
///
/// Devuelve `None` para comandos que **no deben** coalescerse (efectos de borde:
/// disparar SmartGain, conmutar phantom), que siempre se aplican uno a uno.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoalesceKey {
    Gain(u8),
    Mute(u8),
    Phantom(u8),
    Pad(u8),
    Instrument(u8),
    MonitorMix(u8),
    Loopback,
}

impl DeviceCommand {
    /// Clave de agrupación, o `None` si el comando es de efecto de borde.
    pub fn coalesce_key(&self) -> Option<CoalesceKey> {
        match *self {
            DeviceCommand::SetGain { ch, .. } => Some(CoalesceKey::Gain(ch)),
            DeviceCommand::SetMute { ch, .. } => Some(CoalesceKey::Mute(ch)),
            DeviceCommand::SetPhantom { ch, .. } => Some(CoalesceKey::Phantom(ch)),
            DeviceCommand::SetPad { ch, .. } => Some(CoalesceKey::Pad(ch)),
            DeviceCommand::SetInstrument { ch, .. } => Some(CoalesceKey::Instrument(ch)),
            DeviceCommand::SetMonitorMix { out, .. } => Some(CoalesceKey::MonitorMix(out)),
            DeviceCommand::SetLoopback { .. } => Some(CoalesceKey::Loopback),
            DeviceCommand::TriggerSmartGain | DeviceCommand::CancelSmartGain => None,
        }
    }

    /// Canal de entrada afectado, si el comando aplica a uno.
    pub fn channel(&self) -> Option<u8> {
        match *self {
            DeviceCommand::SetGain { ch, .. }
            | DeviceCommand::SetMute { ch, .. }
            | DeviceCommand::SetPhantom { ch, .. }
            | DeviceCommand::SetPad { ch, .. }
            | DeviceCommand::SetInstrument { ch, .. } => Some(ch),
            _ => None,
        }
    }

    /// Etiqueta estable para logs y auditoría.
    pub fn op_name(&self) -> &'static str {
        match self {
            DeviceCommand::SetGain { .. } => "set_gain",
            DeviceCommand::SetMute { .. } => "set_mute",
            DeviceCommand::SetPhantom { .. } => "set_phantom",
            DeviceCommand::SetMonitorMix { .. } => "set_monitor_mix",
            DeviceCommand::SetLoopback { .. } => "set_loopback",
            DeviceCommand::TriggerSmartGain => "trigger_smartgain",
            DeviceCommand::CancelSmartGain => "cancel_smartgain",
            DeviceCommand::SetPad { .. } => "set_pad",
            DeviceCommand::SetInstrument { .. } => "set_instrument",
        }
    }
}
