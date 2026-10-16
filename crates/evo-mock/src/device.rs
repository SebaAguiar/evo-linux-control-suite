//! Dispositivo simulado.

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState, ModelId,
};
use evo_ports::EvoDevice;

/// Fallo programado para el próximo comando.
///
/// Permite testear los caminos de error sin hardware: la desaparencia del
/// dispositivo a mitad de una ráfaga de comandos es justo lo que hay que manejar
/// bien en la reconexión (SAD §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// El dispositivo se desconecta: el comando falla, pasa a `Disconnected` y
    /// se ignora todo comando posterior hasta llamar a [`MockDevice::reconnect`].
    Disconnect,
    /// La transferencia falla de forma transitoria; el dispositivo sigue vivo.
    TransferError(&'static str),
    /// El dispositivo responde con `Degraded` sin fallar el comando.
    Degraded,
}

/// Un EVO simulado que mantiene estado en memoria.
///
/// Se comporta como el hardware real en lo que importa para el núcleo:
/// **valida índices**, devuelve los eventos confirmados y respeta las
/// capacidades del modelo.
pub struct MockDevice {
    profile: DeviceProfile,
    state: DeviceState,
    connection: ConnectionState,
    applied: Vec<DeviceCommand>,
    pending_fault: Option<Fault>,
    /// Si es `false`, se comporta como un dispositivo que no sabe leer su estado
    /// (el caso de v1.0, SAD §8.6).
    can_read_state: bool,
}

impl MockDevice {
    /// Crea un mock con el perfil de un modelo.
    pub fn new(model: ModelId) -> Self {
        let profile = DeviceProfile::for_model(model);
        let state = DeviceState::initial(&profile);
        Self {
            profile,
            state,
            connection: ConnectionState::Connected,
            applied: Vec::new(),
            pending_fault: None,
            can_read_state: false,
        }
    }

    /// Crea un mock con un perfil explícito (para inventar combinaciones de
    /// capacidades que no existen en un modelo real).
    pub fn with_profile(profile: DeviceProfile) -> Self {
        let state = DeviceState::initial(&profile);
        Self {
            profile,
            state,
            connection: ConnectionState::Connected,
            applied: Vec::new(),
            pending_fault: None,
            can_read_state: false,
        }
    }

    /// Habilita `read_state`, como si la Fase 1 hubiese confirmado que el
    /// hardware permite leer registros por control transfer (SAD §8.6).
    pub fn with_read_state(mut self) -> Self {
        self.can_read_state = true;
        self
    }

    /// Programa un fallo para el próximo comando.
    pub fn schedule_fault(&mut self, fault: Fault) -> &mut Self {
        self.pending_fault = Some(fault);
        self
    }

    /// Reconecta un dispositivo caído. Devuelve `false` si ya estaba conectado.
    ///
    /// En el flujo real, la reconexión dispara la reaplicación del último estado
    /// conocido (SAD §8.5, §8.6); eso lo orquesta `evo-core`, no el adaptador.
    pub fn reconnect(&mut self) -> bool {
        if self.connection == ConnectionState::Connected {
            return false;
        }
        self.connection = ConnectionState::Connected;
        true
    }

    /// Comandos aceptados hasta ahora, en orden de llegada.
    ///
    /// Es la herramienta para testear *coalescing*: tras una ráfaga de 200
    /// `SetGain` del mismo canal, aquí deben aparecer muchos menos (SAD §2.2).
    pub fn applied_commands(&self) -> &[DeviceCommand] {
        &self.applied
    }

    /// Vacía el registro de comandos aplicados.
    pub fn clear_applied(&mut self) {
        self.applied.clear();
    }

    /// Estado interno, para aserciones de test.
    pub fn state(&self) -> &DeviceState {
        &self.state
    }
}

impl EvoDevice for MockDevice {
    fn profile(&self) -> &DeviceProfile {
        &self.profile
    }

    fn connection_state(&self) -> ConnectionState {
        self.connection
    }

    fn apply(&mut self, cmd: DeviceCommand) -> Result<Vec<DeviceEvent>, DeviceError> {
        match self.pending_fault.take() {
            Some(Fault::Disconnect) => {
                self.connection = ConnectionState::Disconnected;
                return Err(DeviceError::Disconnected);
            }
            Some(Fault::TransferError(msg)) => return Err(DeviceError::Transfer(msg.to_string())),
            Some(Fault::Degraded) => {
                self.connection = ConnectionState::Degraded;
                return Ok(Vec::new());
            }
            None => {}
        }

        if self.connection == ConnectionState::Disconnected {
            return Err(DeviceError::Disconnected);
        }

        let events = self.apply_validated(cmd)?;
        self.applied.push(cmd);
        Ok(events)
    }

    fn read_state(&mut self) -> Result<DeviceState, DeviceError> {
        if self.can_read_state {
            Ok(self.state.clone())
        } else {
            Err(DeviceError::Unsupported("read_state"))
        }
    }

    fn close(&mut self) -> Result<(), DeviceError> {
        self.connection = ConnectionState::Disconnected;
        Ok(())
    }
}

impl MockDevice {
    /// Aplica el comando al estado interno, validando índices y capacidades como
    /// haría un driver real.
    fn apply_validated(&mut self, cmd: DeviceCommand) -> Result<Vec<DeviceEvent>, DeviceError> {
        let profile = self.profile.clone();
        let mut events = Vec::new();

        match cmd {
            DeviceCommand::SetGain { ch, db } => {
                if !profile.has_input(ch) {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                let db = profile.gain_range_db.clamp(db);
                self.state.channels[ch as usize].gain_db = db;
                events.push(DeviceEvent::GainChanged { ch, db });
            }
            DeviceCommand::SetMute { ch, on } => {
                if !profile.has_input(ch) {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                self.state.channels[ch as usize].muted = on;
                events.push(DeviceEvent::MuteChanged { ch, on });
            }
            DeviceCommand::SetPhantom { ch, on } => {
                if !profile.has_input(ch) {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                self.state.channels[ch as usize].phantom = on;
                events.push(DeviceEvent::PhantomChanged { ch, on });
            }
            DeviceCommand::SetPad { ch, on } => {
                if !profile.has_input(ch) {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                if !profile.supports(evo_domain::Capabilities::INPUT_PAD) {
                    return Err(DeviceError::Unsupported("pad de entrada"));
                }
                self.state.channels[ch as usize].pad = on;
                events.push(DeviceEvent::PadChanged { ch, on });
            }
            DeviceCommand::SetInstrument { ch, on } => {
                if !profile.has_input(ch) {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                if !profile.supports(evo_domain::Capabilities::INSTRUMENT_PER_CHANNEL) {
                    return Err(DeviceError::Unsupported("entrada de instrumento"));
                }
                self.state.channels[ch as usize].instrument = on;
                events.push(DeviceEvent::InstrumentChanged { ch, on });
            }
            DeviceCommand::SetMonitorMix { out, level_db, pan } => {
                if !profile.supports(evo_domain::Capabilities::MONITOR_MIX) {
                    return Err(DeviceError::Unsupported("mezclador de monitor"));
                }
                let Some(mix) = self.state.mixes.get_mut(out as usize) else {
                    return Err(DeviceError::Rejected(format!(
                        "salida {out} fuera de rango"
                    )));
                };
                mix.level_db = level_db;
                mix.pan = pan;
                events.push(DeviceEvent::MonitorMixChanged { out, level_db, pan });
            }
            DeviceCommand::SetLoopback { on } => {
                if !profile.supports(evo_domain::Capabilities::LOOPBACK) {
                    return Err(DeviceError::Unsupported("loop-back"));
                }
                self.state.loopback = on;
                events.push(DeviceEvent::LoopbackChanged { on });
            }
            DeviceCommand::TriggerSmartGain => {
                if !profile.supports(evo_domain::Capabilities::SMARTGAIN) {
                    return Err(DeviceError::Unsupported("SmartGain"));
                }
                self.state.smartgain = evo_domain::SmartGainPhase::Listening;
                events.push(DeviceEvent::SmartGainChanged {
                    phase: evo_domain::SmartGainPhase::Listening,
                });
            }
            DeviceCommand::CancelSmartGain => {
                self.state.smartgain = evo_domain::SmartGainPhase::Cancelled;
                events.push(DeviceEvent::SmartGainChanged {
                    phase: evo_domain::SmartGainPhase::Cancelled,
                });
            }
        }

        Ok(events)
    }
}
