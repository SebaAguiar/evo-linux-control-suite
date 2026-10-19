//! Dispositivo simulado.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState, ModelId,
};
use evo_ports::{EvoDevice, HwEventCallback};

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
    /// Registro compartido y no propio: `evo-core` se queda con el dispositivo
    /// dentro de su hilo, y sin un handle compartido los tests no podrían ver
    /// cuántos comandos llegaron de verdad (que es justo lo que mide el
    /// *coalescing*, SAD §2.2).
    applied: Arc<Mutex<Vec<DeviceCommand>>>,
    /// Fallo programado, compartido para poder dispararlo **después** de
    /// entregar el dispositivo al actor —que es la única forma de simular un
    /// cable que se suelta a mitad de sesión.
    pending_fault: Arc<Mutex<Option<Fault>>>,
    /// Si es `false`, se comporta como un dispositivo que no sabe leer su estado
    /// (el caso de v1.0, SAD §8.6).
    can_read_state: bool,
    /// Latencia simulada por transferencia. El control transfer real del EVO
    /// cuesta ~1 ms (SAD §2.2); modelarla es lo que hace que el *coalescing*
    /// tenga algo que coalescer en los tests.
    apply_delay: Option<Duration>,
    /// Receptor registrado para despachar eventos reactivos de hardware hacia el actor.
    hw_sink: Arc<Mutex<Option<HwEventCallback>>>,
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
            applied: Arc::new(Mutex::new(Vec::new())),
            pending_fault: Arc::new(Mutex::new(None)),
            can_read_state: false,
            apply_delay: None,
            hw_sink: Arc::new(Mutex::new(None)),
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
            applied: Arc::new(Mutex::new(Vec::new())),
            pending_fault: Arc::new(Mutex::new(None)),
            can_read_state: false,
            apply_delay: None,
            hw_sink: Arc::new(Mutex::new(None)),
        }
    }

    /// Simula la latencia de un control transfer real (~1 ms, SAD §2.2).
    pub fn with_apply_delay(mut self, delay: Duration) -> Self {
        self.apply_delay = Some(delay);
        self
    }

    /// Handle compartido al registro de comandos aplicados.
    ///
    /// Se toma **antes** de entregar el dispositivo al actor, y sobrevive a que
    /// este lo posea.
    pub fn log(&self) -> Arc<Mutex<Vec<DeviceCommand>>> {
        Arc::clone(&self.applied)
    }

    /// Añade un registro de comandos compartido (para inspeccionar una
    /// reconexión, donde el dispositivo lo crea una clausura de reapertura).
    pub fn with_log(mut self, log: Arc<Mutex<Vec<DeviceCommand>>>) -> Self {
        self.applied = log;
        self
    }

    /// Habilita `read_state`, como si la Fase 1 hubiese confirmado que el
    /// hardware permite leer registros por control transfer (SAD §8.6).
    pub fn with_read_state(mut self) -> Self {
        self.can_read_state = true;
        self
    }

    /// Programa un fallo para el próximo comando.
    pub fn schedule_fault(&mut self, fault: Fault) -> &mut Self {
        *self.pending_fault.lock().expect("mutex envenenado") = Some(fault);
        self
    }

    /// Handle compartido al fallo programado.
    ///
    /// Permite disparar un fallo **después** de que el actor se haya quedado con
    /// el dispositivo, que es la única forma de simular un cable que se suelta a
    /// mitad de sesión (SAD §8.6).
    pub fn fault_handle(&self) -> Arc<Mutex<Option<Fault>>> {
        Arc::clone(&self.pending_fault)
    }

    /// Consume el fallo programado, si lo hay.
    ///
    /// El fallo no se asocia a un comando concreto: se dispara en el próximo
    /// `apply`, sin importar cuándo llegue.
    fn take_fault(&mut self) -> Option<Fault> {
        self.pending_fault.lock().expect("mutex envenenado").take()
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
    pub fn applied_commands(&self) -> Vec<DeviceCommand> {
        self.applied.lock().expect("mutex envenenado").clone()
    }

    /// Vacía el registro de comandos aplicados.
    pub fn clear_applied(&self) {
        self.applied.lock().expect("mutex envenenado").clear();
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
        match self.take_fault() {
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

        // Latencia simulada del control transfer, antes de tocar el estado: es
        // el tiempo que el actor pasa "fuera", y durante el cual se acumulan los
        // comandos que luego se coalescen (SAD §2.2).
        if let Some(delay) = self.apply_delay {
            std::thread::sleep(delay);
        }

        let events = self.apply_validated(cmd)?;
        self.applied.lock().expect("mutex envenenado").push(cmd);
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

    fn set_hw_event_sink(&mut self, sink: HwEventCallback) {
        *self.hw_sink.lock().expect("mutex envenenado") = Some(sink);
    }
}

impl MockDevice {
    /// Dispara un evento simulado de hardware físico (como si el usuario girara la perilla física o pulsara un botón).
    pub fn trigger_hw_event(&self, event: DeviceEvent) {
        if let Some(sink) = self.hw_sink.lock().expect("mutex envenenado").as_ref() {
            sink(event);
        }
    }

    /// Handle compartido al disparador de eventos de hardware.
    pub fn hw_trigger(&self) -> Arc<Mutex<Option<HwEventCallback>>> {
        Arc::clone(&self.hw_sink)
    }
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
