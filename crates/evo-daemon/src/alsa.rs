//! Adaptador de hardware ALSA para Audient EVO 4.
//!
//! Controla la ganancia de preamplificadores, volumen y switches de la interfaz
//! real a través de los controles expuestos por `snd-usb-audio` en ALSA (SAD §8.1 / §8.7).

use std::process::Command;

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState, ModelId,
    SmartGainPhase,
};
use evo_ports::{DeviceId, DriverFactory, EvoDevice};

/// Comprueba si la tarjeta ALSA del Audient EVO 4 está disponible en el sistema.
pub fn is_evo4_available(card: &str) -> bool {
    Command::new("amixer")
        .args(["-c", card, "cget", "numid=3"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Dispositivo hardware real Audient EVO 4 gobernado vía ALSA.
pub struct Evo4HwDevice {
    card: String,
    profile: DeviceProfile,
    cached_gain_steps: [u32; 4],
    cached_mutes: [bool; 2],
    cached_phantom: [bool; 2],
    loopback: bool,
    smartgain: SmartGainPhase,
}

impl Evo4HwDevice {
    /// Abre el dispositivo comunicando con ALSA para inicializar los niveles actuales.
    pub fn open(card: &str) -> Result<Box<dyn EvoDevice>, DeviceError> {
        if !is_evo4_available(card) {
            return Err(DeviceError::Disconnected);
        }

        let steps = query_mic_volume(card).unwrap_or([0, 0, 0, 0]);

        Ok(Box::new(Self {
            card: card.to_string(),
            profile: DeviceProfile::evo4(),
            cached_gain_steps: steps,
            cached_mutes: [false, false],
            cached_phantom: [false, false],
            loopback: false,
            smartgain: SmartGainPhase::Idle,
        }))
    }
}

impl EvoDevice for Evo4HwDevice {
    fn profile(&self) -> &DeviceProfile {
        &self.profile
    }

    fn connection_state(&self) -> ConnectionState {
        if is_evo4_available(&self.card) {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    fn read_state(&mut self) -> Result<DeviceState, DeviceError> {
        let steps = query_mic_volume(&self.card)?;
        self.cached_gain_steps = steps;

        let mut state = DeviceState::initial(&self.profile);
        state.channels[0].gain_db = step_to_db(steps[0]);
        state.channels[1].gain_db = step_to_db(steps[1]);
        state.channels[0].muted = self.cached_mutes[0];
        state.channels[1].muted = self.cached_mutes[1];
        state.channels[0].phantom = self.cached_phantom[0];
        state.channels[1].phantom = self.cached_phantom[1];
        state.loopback = self.loopback;
        state.smartgain = self.smartgain;

        Ok(state)
    }

    fn apply(&mut self, cmd: DeviceCommand) -> Result<Vec<DeviceEvent>, DeviceError> {
        match cmd {
            DeviceCommand::SetGain { ch, db } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                let step = db_to_step(db);
                self.cached_gain_steps[ch as usize] = step;

                // Si el canal no está muteado, aplicamos inmediatamente al hardware
                if !self.cached_mutes[ch as usize] {
                    write_mic_volume(&self.card, &self.cached_gain_steps)?;
                }
                Ok(vec![DeviceEvent::GainChanged { ch, db }])
            }

            DeviceCommand::SetMute { ch, on } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                self.cached_mutes[ch as usize] = on;

                let mut active_steps = self.cached_gain_steps;
                if self.cached_mutes[0] {
                    active_steps[0] = 0;
                }
                if self.cached_mutes[1] {
                    active_steps[1] = 0;
                }
                write_mic_volume(&self.card, &active_steps)?;
                Ok(vec![DeviceEvent::MuteChanged { ch, on }])
            }

            DeviceCommand::SetPhantom { ch, on } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                self.cached_phantom[ch as usize] = on;
                Ok(vec![DeviceEvent::PhantomChanged { ch, on }])
            }

            DeviceCommand::SetLoopback { on } => {
                self.loopback = on;
                let _ = Command::new("amixer")
                    .args([
                        "-c",
                        &self.card,
                        "cset",
                        "numid=5",
                        if on { "on" } else { "off" },
                    ])
                    .output();
                Ok(vec![DeviceEvent::LoopbackChanged { on }])
            }

            DeviceCommand::SetMonitorMix { out, level_db, pan } => {
                let step = (((level_db + 127.0) / 0.5).round() as i32).clamp(0, 254) as u32;
                let _ = Command::new("amixer")
                    .args([
                        "-c",
                        &self.card,
                        "cset",
                        "numid=4",
                        &format!("{step},{step},0,0"),
                    ])
                    .output();
                Ok(vec![DeviceEvent::MonitorMixChanged { out, level_db, pan }])
            }

            DeviceCommand::TriggerSmartGain => {
                self.smartgain = SmartGainPhase::Listening;
                Ok(vec![DeviceEvent::SmartGainChanged {
                    phase: SmartGainPhase::Listening,
                }])
            }

            DeviceCommand::CancelSmartGain => {
                self.smartgain = SmartGainPhase::Idle;
                Ok(vec![DeviceEvent::SmartGainChanged {
                    phase: SmartGainPhase::Idle,
                }])
            }

            other => Err(DeviceError::Unsupported(other.op_name())),
        }
    }
}

/// Convierte dB (-8 a 50 dB) a pasos discretos de ALSA (0 a 116, 0.5 dB por paso).
fn db_to_step(db: f32) -> u32 {
    let step = ((db + 8.0) / 0.5).round() as i32;
    step.clamp(0, 116) as u32
}

/// Convierte pasos discretos de ALSA (0 a 116) a dB.
fn step_to_db(step: u32) -> f32 {
    (step as f32) * 0.5 - 8.0
}

/// Lee los pasos de volumen de micrófono desde ALSA `numid=3`.
fn query_mic_volume(card: &str) -> Result<[u32; 4], DeviceError> {
    let output = Command::new("amixer")
        .args(["-c", card, "cget", "numid=3"])
        .output()
        .map_err(|e| DeviceError::Transfer(e.to_string()))?;

    if !output.status.success() {
        return Err(DeviceError::Disconnected);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(": values=") {
            let parts: Vec<u32> = rest
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .collect();
            if parts.len() >= 4 {
                return Ok([parts[0], parts[1], parts[2], parts[3]]);
            }
        }
    }

    Err(DeviceError::Transfer(
        "No se pudo parsear numid=3 de ALSA".into(),
    ))
}

/// Envía los 4 valores de pasos al control ALSA `numid=3`.
fn write_mic_volume(card: &str, steps: &[u32; 4]) -> Result<(), DeviceError> {
    let arg = format!("{},{},{},{}", steps[0], steps[1], steps[2], steps[3]);
    let status = Command::new("amixer")
        .args(["-c", card, "cset", "numid=3", &arg])
        .status()
        .map_err(|e| DeviceError::Transfer(e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(DeviceError::Disconnected)
    }
}

/// Fábrica para registrar el hardware Audient EVO 4.
pub struct Evo4AlsaDriverFactory {
    card: String,
    profile: DeviceProfile,
}

impl Evo4AlsaDriverFactory {
    pub fn new(card: impl Into<String>) -> Self {
        Self {
            card: card.into(),
            profile: DeviceProfile::evo4(),
        }
    }
}

impl DriverFactory for Evo4AlsaDriverFactory {
    fn model(&self) -> ModelId {
        ModelId::Evo4
    }

    fn profile(&self) -> DeviceProfile {
        self.profile.clone()
    }

    fn device_id(&self) -> Option<DeviceId> {
        // Audient EVO 4 confirmado en Linux: VID=0x2708, PID=0x0006
        Some(DeviceId::new(0x2708, 0x0006))
    }

    fn open(&self) -> Result<Box<dyn EvoDevice>, DeviceError> {
        Evo4HwDevice::open(&self.card)
    }
}
