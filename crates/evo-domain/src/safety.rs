//! `SafetyPolicy`: la defensa principal contra comandos que dañan equipos.
//!
//! El atributo de calidad número uno del proyecto es **la seguridad del hardware
//! y del audio** (SAD §0): nunca enviar un comando que pueda dañar equipos
//! conectados. Por eso la política vive en el núcleo puro, se aplica a **todos**
//! los orígenes (escritorio, móvil y presets) y **ante la duda rechaza**
//! (SAD §4.3, §8.4).
//!
//! Es determinista y sin I/O: no consulta el reloj. El tiempo entra como
//! parámetro (`now`, un reloj monótono), así que el *cooldown* de phantom es
//! testeable sin `sleep`s ni flakiness.

use crate::command::DeviceCommand;
use crate::error::{CommandRejection, DomainError, SafetyError};
use crate::profile::{Capabilities, DeviceProfile, GainRange};
use crate::state::DeviceState;
use std::time::Duration;

/// Límites configurables de la política de seguridad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafetyLimits {
    /// Ventana mínima entre conmutaciones de phantom, por canal.
    ///
    /// Evita *pops* audibles y el estrés del micrófono al ciclar +48 V
    /// (SAD §4.3).
    pub phantom_cooldown: Duration,
    /// Rango permitido para el nivel de un bus de monitorización.
    ///
    /// **Provisional**, a confirmar con el hardware en Fase 1.
    pub monitor_level_db: GainRange,
    /// Rango permitido de paneo.
    pub pan_range: GainRange,
}

impl Default for SafetyLimits {
    fn default() -> Self {
        Self {
            phantom_cooldown: Duration::from_millis(2_000),
            monitor_level_db: GainRange::new(-127.0, 6.0),
            pan_range: GainRange::new(-1.0, 1.0),
        }
    }
}

/// Estado externo que la política necesita para decidir.
///
/// Lo mantiene el `evo-core` (no el dominio): el dominio no guarda estado
/// mutable propio, solo evalúa.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SafetyContext {
    /// Bloqueo global de phantom desde el escritorio (SAD §4.3).
    pub phantom_locked: bool,
    /// Instante del último cambio de phantom por canal, en el reloj monótono.
    /// Índice = canal; `None` = nunca se cambió.
    pub last_phantom_change_at: [Option<Duration>; crate::MAX_CHANNELS],
    /// ¿Hay ya un SmartGain en curso? Solo se permite una ejecución concurrente.
    pub smartgain_running: bool,
}

/// Política de seguridad del núcleo.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SafetyPolicy {
    limits: SafetyLimits,
}

impl SafetyPolicy {
    pub const fn new(limits: SafetyLimits) -> Self {
        Self { limits }
    }

    pub const fn limits(&self) -> &SafetyLimits {
        &self.limits
    }

    /// Valida y **normaliza** un comando antes de que toque el hardware.
    ///
    /// Devuelve el comando que realmente debe aplicarse: puede diferir del de
    /// entrada cuando el valor se ajusta al rango (`clamp` de ganancia). Valores
    /// no finitos se **rechazan**, no se ajustan.
    ///
    /// `now` es el instante actual en un reloj monótono, usado para el
    /// *cooldown* de phantom.
    pub fn validate(
        &self,
        cmd: DeviceCommand,
        profile: &DeviceProfile,
        state: &DeviceState,
        ctx: &SafetyContext,
        now: Duration,
    ) -> Result<DeviceCommand, CommandRejection> {
        match cmd {
            DeviceCommand::SetGain { ch, db } => {
                let ch = require_input(profile, ch)?;
                require_finite("gain_db", db)?;
                Ok(DeviceCommand::SetGain {
                    ch,
                    db: profile.gain_range_db.clamp(db),
                })
            }

            DeviceCommand::SetMute { ch, on } => {
                let ch = require_input(profile, ch)?;
                require_capability(profile, Capabilities::MUTE_PER_CHANNEL, "mute por canal")?;
                Ok(DeviceCommand::SetMute { ch, on })
            }

            DeviceCommand::SetPhantom { ch, on } => {
                let ch = require_input(profile, ch)?;
                require_capability(
                    profile,
                    Capabilities::PHANTOM_PER_CHANNEL,
                    "phantom por canal",
                )?;

                let already = state.channels[ch as usize].phantom;
                if already == on {
                    // Idempotente y sin efecto: no se consume cooldown ni se
                    // genera un pop innecesario (SAD §4.3).
                    return Ok(DeviceCommand::SetPhantom { ch, on });
                }
                if ctx.phantom_locked {
                    return Err(SafetyError::PhantomLocked.into());
                }
                if let Some(last) = ctx.last_phantom_change_at[ch as usize] {
                    let elapsed = now.saturating_sub(last);
                    if elapsed < self.limits.phantom_cooldown {
                        return Err(SafetyError::PhantomCooldown {
                            remaining_ms: (self.limits.phantom_cooldown - elapsed).as_millis()
                                as u64,
                        }
                        .into());
                    }
                }
                Ok(DeviceCommand::SetPhantom { ch, on })
            }

            DeviceCommand::SetMonitorMix { out, level_db, pan } => {
                require_output(profile, out)?;
                require_capability(profile, Capabilities::MONITOR_MIX, "mezclador de monitor")?;
                require_finite("level_db", level_db)?;
                require_finite("pan", pan)?;
                Ok(DeviceCommand::SetMonitorMix {
                    out,
                    level_db: self.limits.monitor_level_db.clamp(level_db),
                    pan: self.limits.pan_range.clamp(pan),
                })
            }

            DeviceCommand::SetLoopback { on } => {
                require_capability(profile, Capabilities::LOOPBACK, "loop-back")?;
                Ok(DeviceCommand::SetLoopback { on })
            }

            DeviceCommand::SetPad { ch, on } => {
                let ch = require_input(profile, ch)?;
                require_capability(profile, Capabilities::INPUT_PAD, "pad de entrada")?;
                Ok(DeviceCommand::SetPad { ch, on })
            }

            DeviceCommand::SetInstrument { ch, on } => {
                let ch = require_input(profile, ch)?;
                require_capability(
                    profile,
                    Capabilities::INSTRUMENT_PER_CHANNEL,
                    "entrada de instrumento",
                )?;
                Ok(DeviceCommand::SetInstrument { ch, on })
            }

            DeviceCommand::TriggerSmartGain => {
                require_capability(profile, Capabilities::SMARTGAIN, "SmartGain")?;
                if ctx.smartgain_running {
                    return Err(SafetyError::SmartGainBusy.into());
                }
                Ok(DeviceCommand::TriggerSmartGain)
            }

            // Cancelar siempre es seguro: es la operación que *quita* energía
            // del sistema.
            DeviceCommand::CancelSmartGain => Ok(DeviceCommand::CancelSmartGain),
        }
    }
}

fn require_input(profile: &DeviceProfile, ch: u8) -> Result<u8, DomainError> {
    if profile.has_input(ch) {
        Ok(ch)
    } else {
        Err(DomainError::ChannelOutOfRange {
            ch,
            channels: profile.inputs,
        })
    }
}

fn require_output(profile: &DeviceProfile, out: u8) -> Result<u8, DomainError> {
    if profile.has_output(out) {
        Ok(out)
    } else {
        Err(DomainError::OutputOutOfRange {
            out,
            outputs: profile.outputs,
        })
    }
}

fn require_finite(param: &'static str, value: f32) -> Result<(), DomainError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DomainError::NotFinite { param })
    }
}

fn require_capability(
    profile: &DeviceProfile,
    cap: Capabilities,
    feature: &'static str,
) -> Result<(), SafetyError> {
    if profile.supports(cap) {
        Ok(())
    } else {
        Err(SafetyError::Unsupported { feature })
    }
}
