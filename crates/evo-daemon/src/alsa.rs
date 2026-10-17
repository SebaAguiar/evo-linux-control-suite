//! Adaptador de hardware ALSA y USB crudo para Audient EVO 4.
//!
//! Controla la ganancia de preamplificadores, volumen, phantom 48V y mute
//! físico de la interfaz real a través de:
//! 1. `/dev/evo4` (vía módulo de kernel `evo_raw`): control USB directo a nivel
//!    de hardware para 48V (relé físico), mute real (botón titilante) y monitor mix.
//! 2. ALSA `snd-usb-audio`: para gain de preamps (`Mic Playback Volume`, numid=3)
//!    y volumen de salida (`EVO4 Playback Volume`, numid=4).

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::Command;

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState, ModelId,
    SmartGainPhase,
};
use evo_ports::{DeviceId, DriverFactory, EvoDevice};

// --- Protocolo de ioctl para /dev/evo4 (evo_raw) ---

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EvoCtrlXfer {
    pub b_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
    pub data: [u8; 256],
}

impl Default for EvoCtrlXfer {
    fn default() -> Self {
        Self {
            b_request_type: 0,
            b_request: 0,
            w_value: 0,
            w_index: 0,
            w_length: 0,
            data: [0; 256],
        }
    }
}

const EVO_IOC_DIR_RW: u32 = 3;
const EVO_IOC_SIZE: u32 = std::mem::size_of::<EvoCtrlXfer>() as u32; // 264
const EVO_IOC_TYPE: u32 = 0x45; // 'E'
const EVO_IOC_NR: u32 = 0;
const EVO_CTRL_TRANSFER: libc::c_ulong =
    ((EVO_IOC_DIR_RW << 30) | (EVO_IOC_SIZE << 16) | (EVO_IOC_TYPE << 8) | EVO_IOC_NR)
        as libc::c_ulong;

/// Envía una solicitud SET_CUR (Host -> Device) vía `/dev/evo4`.
pub fn evo_raw_set_cur(
    file: &File,
    w_value: u16,
    w_index: u16,
    data: &[u8],
) -> std::io::Result<()> {
    let mut xfer = EvoCtrlXfer {
        b_request_type: 0x21, // REQTYPE_SET: Host->Device, Class, Interface
        b_request: 0x01,      // SET_CUR
        w_value,
        w_index,
        w_length: data.len() as u16,
        data: [0; 256],
    };
    let copy_len = data.len().min(256);
    xfer.data[..copy_len].copy_from_slice(&data[..copy_len]);

    let ret = unsafe { libc::ioctl(file.as_raw_fd(), EVO_CTRL_TRANSFER, &mut xfer) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Envía una solicitud GET_CUR (Device -> Host) vía `/dev/evo4`.
pub fn evo_raw_get_cur(
    file: &File,
    w_value: u16,
    w_index: u16,
    len: usize,
) -> std::io::Result<Vec<u8>> {
    let mut xfer = EvoCtrlXfer {
        b_request_type: 0xA1, // REQTYPE_GET: Device->Host, Class, Interface
        b_request: 0x01,      // GET_CUR
        w_value,
        w_index,
        w_length: len as u16,
        data: [0; 256],
    };

    let ret = unsafe { libc::ioctl(file.as_raw_fd(), EVO_CTRL_TRANSFER, &mut xfer) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        let resp_len = (xfer.w_length as usize).min(256);
        Ok(xfer.data[..resp_len].to_vec())
    }
}

/// Comprueba si la tarjeta ALSA o el dispositivo /dev/evo4 del Audient EVO 4 están disponibles.
pub fn is_evo4_available(card: &str) -> bool {
    Path::new("/dev/evo4").exists()
        || Command::new("amixer")
            .args(["-c", card, "cget", "numid=3"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Dispositivo hardware real Audient EVO 4.
pub struct Evo4HwDevice {
    card: String,
    profile: DeviceProfile,
    cached_gain_steps: [u32; 4],
    cached_mutes: [bool; 2],
    cached_phantom: [bool; 2],
    cached_volume_step: u32,
    cached_mix_raw: u8,
    loopback: bool,
    smartgain: SmartGainPhase,
    raw_dev: Option<File>,
}

impl Evo4HwDevice {
    /// Abre el dispositivo comunicando con ALSA y opcionalmente `/dev/evo4` si está presente.
    pub fn open(card: &str) -> Result<Box<dyn EvoDevice>, DeviceError> {
        if !is_evo4_available(card) {
            return Err(DeviceError::Disconnected);
        }

        let raw_dev = match OpenOptions::new().read(true).write(true).open("/dev/evo4") {
            Ok(file) => {
                tracing::info!(
                    "Dispositivo /dev/evo4 abierto: control nativo de 48V y mute físico disponible."
                );
                Some(file)
            }
            Err(e) => {
                tracing::warn!(
                    "No se pudo abrir /dev/evo4 ({e}): operando en modo ALSA básico. \
                     Para activar 48V físico y botón mute titilante, ejecute: sudo bash driver/evo_raw/install.sh"
                );
                None
            }
        };

        let steps = query_mic_volume(card).unwrap_or([0, 0, 0, 0]);
        let volume_step = query_playback_volume(card).unwrap_or(254);

        let mut dev = Self {
            card: card.to_string(),
            profile: DeviceProfile::evo4(),
            cached_gain_steps: steps,
            cached_mutes: [false, false],
            cached_phantom: [false, false],
            cached_volume_step: volume_step,
            cached_mix_raw: 64, // 50/50 balance inicial
            loopback: false,
            smartgain: SmartGainPhase::Idle,
            raw_dev,
        };

        // Si tenemos acceso crudo al hardware, leemos el estado físico real
        if let Some(file) = &dev.raw_dev {
            if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3A00, 4) {
                if !data.is_empty() {
                    dev.cached_phantom[0] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0001, 0x3A00, 4) {
                if !data.is_empty() {
                    dev.cached_phantom[1] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0200, 0x3A00, 4) {
                if !data.is_empty() {
                    dev.cached_mutes[0] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0201, 0x3A00, 4) {
                if !data.is_empty() {
                    dev.cached_mutes[1] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3800, 2) {
                if !data.is_empty() {
                    dev.cached_mix_raw = data[0].clamp(0, 127);
                }
            }
        }

        Ok(Box::new(dev))
    }

    /// Asegura que `/dev/evo4` esté abierto si no lo está.
    /// Si el dispositivo se había desconectado y reaparece el nodo en devfs, lo reabre.
    fn ensure_raw_dev(&mut self) {
        if self.raw_dev.is_none() && Path::new("/dev/evo4").exists() {
            match OpenOptions::new().read(true).write(true).open("/dev/evo4") {
                Ok(file) => {
                    tracing::info!(
                        "Dispositivo /dev/evo4 detectado y abierto: control nativo de 48V y mute físico disponible."
                    );
                    self.raw_dev = Some(file);
                }
                Err(e) => {
                    tracing::warn!("Se detectó /dev/evo4 pero no se pudo abrir ({e})");
                }
            }
        }
    }
}

fn is_stale_fd(err: &std::io::Error) -> bool {
    let code = err.raw_os_error().unwrap_or(0);
    code == libc::ENODEV
        || code == libc::EBADF
        || code == libc::EIO
        || code == libc::ENXIO
        || code == libc::EPROTO
        || code == libc::ESHUTDOWN
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
        // Intentamos detectar /dev/evo4 o reabrirlo si se reconectó el USB
        self.ensure_raw_dev();

        // Si tenemos /dev/evo4, consultamos el estado físico real de volumen, botones y relés directamente
        if let Some(file) = &self.raw_dev {
            let mut stale = false;
            for ch in 0..2 {
                std::thread::sleep(std::time::Duration::from_millis(3));
                match evo_raw_get_cur(file, (2 << 8) | ((ch + 1) as u16), 0x0B00, 2) {
                    Ok(data) => {
                        if data.len() >= 2 {
                            let raw = i16::from_le_bytes([data[0], data[1]]);
                            let db = (raw as f32) / 256.0;
                            self.cached_gain_steps[ch] = db_to_step(db);
                        }
                    }
                    Err(e) if is_stale_fd(&e) => {
                        stale = true;
                        break;
                    }
                    Err(_) => {}
                }
            }
            if stale {
                tracing::warn!(
                    "El descriptor de /dev/evo4 quedó obsoleto tras desconexión USB; cerrando."
                );
                self.raw_dev = None;
            } else {
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3A00, 4) {
                    if !data.is_empty() {
                        self.cached_phantom[0] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0001, 0x3A00, 4) {
                    if !data.is_empty() {
                        self.cached_phantom[1] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0200, 0x3A00, 4) {
                    if !data.is_empty() {
                        self.cached_mutes[0] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0201, 0x3A00, 4) {
                    if !data.is_empty() {
                        self.cached_mutes[1] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3800, 2) {
                    if !data.is_empty() {
                        self.cached_mix_raw = data[0].clamp(0, 127);
                    }
                }
            }
        } else {
            // Modo fallback ALSA básico: actualizamos pasos de ganancia desde ALSA
            if let Ok(steps) = query_mic_volume(&self.card) {
                for (ch, &step) in steps.iter().enumerate().take(2) {
                    if self.cached_mutes[ch] {
                        continue;
                    }
                    self.cached_gain_steps[ch] = step;
                }
                if self.cached_gain_steps.len() > 2 {
                    self.cached_gain_steps[2] = steps[2];
                    self.cached_gain_steps[3] = steps[3];
                }
            }
        }

        // Actualizamos volumen de salida desde ALSA numid=4
        if let Ok(vol) = query_playback_volume(&self.card) {
            self.cached_volume_step = vol;
        }

        let mut state = DeviceState::initial(&self.profile);
        state.channels[0].gain_db = step_to_db(self.cached_gain_steps[0]);
        state.channels[1].gain_db = step_to_db(self.cached_gain_steps[1]);
        state.channels[0].muted = self.cached_mutes[0];
        state.channels[1].muted = self.cached_mutes[1];
        state.channels[0].phantom = self.cached_phantom[0];
        state.channels[1].phantom = self.cached_phantom[1];
        state.loopback = self.loopback;
        state.smartgain = self.smartgain;

        let vol_db = (self.cached_volume_step as f32) * 0.5 - 127.0;
        let pan = ((self.cached_mix_raw as f32) / 127.0) * 2.0 - 1.0;
        for mix in &mut state.mixes {
            mix.level_db = vol_db;
            mix.pan = pan;
        }

        Ok(state)
    }

    fn apply(&mut self, cmd: DeviceCommand) -> Result<Vec<DeviceEvent>, DeviceError> {
        self.ensure_raw_dev();
        match cmd {
            DeviceCommand::SetGain { ch, db } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                let step = db_to_step(db);
                self.cached_gain_steps[ch as usize] = step;

                // Si tenemos /dev/evo4 escribimos directo al Feature Unit 11 CS=2
                if let Some(file) = &self.raw_dev {
                    let raw = (db * 256.0).round() as i16;
                    if let Err(e) = evo_raw_set_cur(
                        file,
                        (2 << 8) | ((ch + 1) as u16),
                        0x0B00,
                        &raw.to_le_bytes(),
                    ) {
                        if is_stale_fd(&e) {
                            self.raw_dev = None;
                        }
                    }
                    let _ = write_mic_volume(&self.card, &self.cached_gain_steps);
                } else {
                    // En modo ALSA básico, si está muteado mantenemos 0 en el control ALSA
                    let mut active_steps = self.cached_gain_steps;
                    if self.cached_mutes[0] {
                        active_steps[0] = 0;
                    }
                    if self.cached_mutes[1] {
                        active_steps[1] = 0;
                    }
                    let _ = write_mic_volume(&self.card, &active_steps);
                }

                Ok(vec![DeviceEvent::GainChanged { ch, db }])
            }

            DeviceCommand::SetMute { ch, on } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                self.cached_mutes[ch as usize] = on;

                // Si tenemos /dev/evo4, activamos el mute nativo por hardware en EU58 CS=2.
                // Esto hace que el hardware silencie la entrada y el LED del botón del canal parpadee.
                if let Some(file) = &self.raw_dev {
                    let val: [u8; 4] = [if on { 1 } else { 0 }, 0, 0, 0];
                    if let Err(e) = evo_raw_set_cur(file, 0x0200 + (ch as u16), 0x3A00, &val) {
                        tracing::error!("Error enviando mute físico a /dev/evo4: {e}");
                        if is_stale_fd(&e) {
                            self.raw_dev = None;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                } else {
                    // Fallback en modo ALSA básico: simular mute bajando el paso a 0
                    let mut active_steps = self.cached_gain_steps;
                    if self.cached_mutes[0] {
                        active_steps[0] = 0;
                    }
                    if self.cached_mutes[1] {
                        active_steps[1] = 0;
                    }
                    let _ = write_mic_volume(&self.card, &active_steps);
                }

                Ok(vec![DeviceEvent::MuteChanged { ch, on }])
            }

            DeviceCommand::SetPhantom { ch, on } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }

                // En el hardware Audient EVO 4, 48V es controlado por EU58 CS=0.
                // Requiere /dev/evo4 para disparar el relé físico y encender el LED de 48V.
                if let Some(file) = &self.raw_dev {
                    let val: [u8; 4] = [if on { 1 } else { 0 }, 0, 0, 0];
                    if let Err(e) = evo_raw_set_cur(file, ch as u16, 0x3A00, &val) {
                        tracing::error!("Error activando 48V físico en /dev/evo4: {e}");
                        if is_stale_fd(&e) {
                            self.raw_dev = None;
                        }
                        return Err(DeviceError::Transfer(e.to_string()));
                    }
                    // Breve pausa para estabilizar los contactos mecánicos del relé
                    // y el convertidor elevador DC-DC antes de reanudar el tráfico USB.
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    self.cached_phantom[ch as usize] = on;
                    Ok(vec![DeviceEvent::PhantomChanged { ch, on }])
                } else {
                    tracing::warn!(
                        "SetPhantom: /dev/evo4 no disponible. El relé de 48V físico requiere el módulo evo_raw. \
                         Ejecute 'sudo bash driver/evo_raw/install.sh'."
                    );
                    Err(DeviceError::Unsupported(
                        "El relé de +48V físico requiere el módulo evo_raw. Ejecute: sudo bash driver/evo_raw/install.sh",
                    ))
                }
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
                // 1. Control de volumen general de salida (ALSA numid=4 'EVO4 Playback Volume', 0 a 254 pasos)
                let step = (((level_db + 127.0) / 0.5).round() as i32).clamp(0, 254) as u32;
                self.cached_volume_step = step;
                let _ = write_playback_volume(&self.card, step);

                // 2. Mix entre PC y placa (Extension Unit 56 CS=0 en /dev/evo4):
                // pan en [-1.0, 1.0] -> 0 = 100% entrada/placa, 127 = 100% PC/DAW, 64 = 50/50 balance
                let raw_mix = (((pan + 1.0) / 2.0) * 127.0).round().clamp(0.0, 127.0) as u8;
                self.cached_mix_raw = raw_mix;

                if let Some(file) = &self.raw_dev {
                    if let Err(e) = evo_raw_set_cur(file, 0x0000, 0x3800, &[raw_mix, 0]) {
                        if is_stale_fd(&e) {
                            self.raw_dev = None;
                        }
                    }
                }

                Ok(vec![DeviceEvent::MonitorMixChanged { out, level_db, pan }])
            }

            DeviceCommand::TriggerSmartGain | DeviceCommand::CancelSmartGain => {
                // En Audient EVO 4, el proceso de SmartGain se ejecuta por hardware
                // en el microcontrolador de la interfaz mediante la secuencia física de botones:
                // 1) Pulsar botón verde SmartGain (los botones 1 y 2 parpadean).
                // 2) Tocar canal 1 o 2 para seleccionarlo.
                // 3) Pulsar SmartGain de nuevo para iniciar la calibración.
                // Al finalizar, la ganancia calibrada en FU11 es leída automáticamente por el daemon.
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

/// Lee los pasos de volumen de salida general desde ALSA `numid=4` ('EVO4 Playback Volume', 0 a 254).
fn query_playback_volume(card: &str) -> Result<u32, DeviceError> {
    let output = Command::new("amixer")
        .args(["-c", card, "cget", "numid=4"])
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
            if !parts.is_empty() {
                return Ok(parts[0]);
            }
        }
    }

    Err(DeviceError::Transfer(
        "No se pudo parsear numid=4 de ALSA".into(),
    ))
}

/// Envía el paso de volumen general al control ALSA `numid=4`.
fn write_playback_volume(card: &str, step: u32) -> Result<(), DeviceError> {
    let arg = format!("{step},{step},0,0");
    let status = Command::new("amixer")
        .args(["-c", card, "cset", "numid=4", &arg])
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
