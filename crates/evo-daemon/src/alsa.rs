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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState, ModelId,
    SmartGainPhase,
};
use evo_ports::{DeviceId, DriverFactory, EvoDevice, HwEventCallback};

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

// --- Protocolo de ioctl para ALSA Control (/dev/snd/controlC*) ---
const SNDRV_CTL_IOCTL_SUBSCRIBE_EVENTS: libc::c_ulong = 0xc0045516;
const SNDRV_CTL_IOCTL_ELEM_READ: libc::c_ulong = 0xc4c85512;
const SNDRV_CTL_EVENT_MASK_VALUE: u32 = 0x01;
const SNDRV_CTL_EVENT_ELEM: i32 = 0;

/// Busca la ruta del dispositivo de control ALSA para una tarjeta dada (ej. "EVO4", "1", "hw:1").
pub fn find_control_dev_node(card: &str) -> Option<String> {
    let clean_card = card.strip_prefix("hw:").unwrap_or(card);
    if clean_card.chars().all(|c| c.is_ascii_digit()) {
        let p = format!("/dev/snd/controlC{clean_card}");
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    let asound_card = format!("/proc/asound/{clean_card}");
    if let Ok(target) = std::fs::read_link(&asound_card) {
        if let Some(target_str) = target.to_str() {
            if let Some(num) = target_str.strip_prefix("card") {
                let p = format!("/dev/snd/controlC{num}");
                if Path::new(&p).exists() {
                    return Some(p);
                }
            }
        }
    }
    if let Ok(content) = std::fs::read_to_string("/proc/asound/cards") {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((idx_str, rest)) = trimmed.split_once('[') {
                if rest.contains(clean_card) {
                    if let Ok(idx) = idx_str.trim().parse::<u32>() {
                        let p = format!("/dev/snd/controlC{idx}");
                        if Path::new(&p).exists() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    if clean_card.starts_with("/dev/snd/controlC") && Path::new(clean_card).exists() {
        return Some(clean_card.to_string());
    }
    None
}

/// Lee los valores enteros de un control ALSA por su numid de forma instantánea vía ioctl.
fn alsa_read_numid(fd: libc::c_int, numid: u32) -> std::io::Result<[i64; 2]> {
    let mut buf = [0u8; 1224];
    buf[0..4].copy_from_slice(&numid.to_ne_bytes());
    let ret = unsafe { libc::ioctl(fd, SNDRV_CTL_IOCTL_ELEM_READ, buf.as_mut_ptr()) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let val0 = i64::from_ne_bytes(buf[72..80].try_into().unwrap());
    let val1 = i64::from_ne_bytes(buf[80..88].try_into().unwrap());
    Ok([val0, val1])
}

/// Estado compartido entre el actor y el hilo listener reactivo de ALSA.
#[derive(Debug, Default)]
pub struct EvoSharedState {
    pub gain_steps: [u32; 4],
    pub volume_step: u32,
    pub mutes: [bool; 2],
    pub phantom: [bool; 2],
    pub mix_raw: u8,
}

/// Hilo en segundo plano que escucha eventos del kernel emitidos en `/dev/snd/controlC*`.
fn run_alsa_listener(
    dev_path: String,
    stop: Arc<AtomicBool>,
    sink_holder: Arc<Mutex<Option<HwEventCallback>>>,
    shared: Arc<Mutex<EvoSharedState>>,
) {
    let file = match OpenOptions::new().read(true).write(true).open(&dev_path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("No se pudo abrir {dev_path} para suscripción de eventos ALSA: {e}");
            return;
        }
    };
    let fd = file.as_raw_fd();
    let mut subscribe: libc::c_int = 1;
    let ret = unsafe { libc::ioctl(fd, SNDRV_CTL_IOCTL_SUBSCRIBE_EVENTS, &mut subscribe) };
    if ret < 0 {
        tracing::warn!(
            "SNDRV_CTL_IOCTL_SUBSCRIBE_EVENTS falló en {dev_path}: {}",
            std::io::Error::last_os_error()
        );
        return;
    }
    tracing::info!("Listener reactivo de eventos ALSA iniciado en {dev_path}");

    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    while !stop.load(Ordering::Relaxed) {
        pollfd.revents = 0;
        let poll_ret = unsafe { libc::poll(&mut pollfd, 1, 300) };
        if poll_ret <= 0 {
            continue;
        }
        if (pollfd.revents & libc::POLLIN) == 0 {
            if (pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                tracing::warn!("Descriptor {dev_path} cerrado o desconectado en ALSA listener");
                break;
            }
            continue;
        }

        let mut event_buf = [0u8; 72];
        let n = unsafe { libc::read(fd, event_buf.as_mut_ptr() as *mut libc::c_void, 72) };
        if n != 72 {
            continue;
        }

        let event_type = i32::from_ne_bytes(event_buf[0..4].try_into().unwrap());
        let mask = u32::from_ne_bytes(event_buf[4..8].try_into().unwrap());
        let numid = u32::from_ne_bytes(event_buf[8..12].try_into().unwrap());

        if event_type == SNDRV_CTL_EVENT_ELEM && (mask & SNDRV_CTL_EVENT_MASK_VALUE) != 0 {
            if numid == 3 {
                // Mic Playback Volume (Ganancia canales 1 y 2)
                if let Ok([val0, val1]) = alsa_read_numid(fd, 3) {
                    let step0 = (val0.clamp(0, 50)) as u32;
                    let step1 = (val1.clamp(0, 50)) as u32;

                    let mut changed_events = Vec::new();
                    {
                        let mut state = shared.lock().unwrap();
                        if state.gain_steps[0] != step0 && !state.mutes[0] {
                            state.gain_steps[0] = step0;
                            let db = step_to_db(step0);
                            changed_events.push(DeviceEvent::GainChanged { ch: 0, db });
                        }
                        if state.gain_steps[1] != step1 && !state.mutes[1] {
                            state.gain_steps[1] = step1;
                            let db = step_to_db(step1);
                            changed_events.push(DeviceEvent::GainChanged { ch: 1, db });
                        }
                    }

                    if !changed_events.is_empty() {
                        if let Some(sink) = sink_holder.lock().unwrap().as_ref() {
                            for ev in changed_events {
                                sink(ev);
                            }
                        }
                    }
                }
            } else if numid == 4 {
                // EVO4 Playback Volume (Volumen general de salida)
                if let Ok([vol0, _]) = alsa_read_numid(fd, 4) {
                    let vol_step = (vol0.clamp(0, 254)) as u32;
                    let mut emit_event = None;
                    {
                        let mut state = shared.lock().unwrap();
                        if state.volume_step != vol_step {
                            state.volume_step = vol_step;
                            let vol_db = (vol_step as f32) * 0.5 - 127.0;
                            let pan = ((state.mix_raw as f32) / 127.0) * 2.0 - 1.0;
                            emit_event = Some(DeviceEvent::MonitorMixChanged {
                                out: 0,
                                level_db: vol_db,
                                pan,
                            });
                        }
                    }
                    if let Some(ev) = emit_event {
                        if let Some(sink) = sink_holder.lock().unwrap().as_ref() {
                            sink(ev);
                        }
                    }
                }
            }
        }
    }
    tracing::info!("Listener reactivo de eventos ALSA finalizado en {dev_path}");
}

/// Dispositivo hardware real Audient EVO 4.
pub struct Evo4HwDevice {
    card: String,
    profile: DeviceProfile,
    shared: Arc<Mutex<EvoSharedState>>,
    loopback: bool,
    smartgain: SmartGainPhase,
    raw_dev: Option<File>,
    hw_sink: Arc<Mutex<Option<HwEventCallback>>>,
    listener_stop: Arc<AtomicBool>,
    listener_thread: Option<std::thread::JoinHandle<()>>,
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

        let shared = Arc::new(Mutex::new(EvoSharedState {
            gain_steps: steps,
            volume_step,
            mutes: [false, false],
            phantom: [false, false],
            mix_raw: 64, // 50/50 balance inicial
        }));

        let dev = Self {
            card: card.to_string(),
            profile: DeviceProfile::evo4(),
            shared,
            loopback: false,
            smartgain: SmartGainPhase::Idle,
            raw_dev,
            hw_sink: Arc::new(Mutex::new(None)),
            listener_stop: Arc::new(AtomicBool::new(false)),
            listener_thread: None,
        };

        // Si tenemos acceso crudo al hardware, leemos el estado físico real
        if let Some(file) = &dev.raw_dev {
            let mut state = dev.shared.lock().unwrap();
            if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3A00, 4) {
                if !data.is_empty() {
                    state.phantom[0] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0001, 0x3A00, 4) {
                if !data.is_empty() {
                    state.phantom[1] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0200, 0x3A00, 4) {
                if !data.is_empty() {
                    state.mutes[0] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0201, 0x3A00, 4) {
                if !data.is_empty() {
                    state.mutes[1] = data[0] == 1;
                }
            }
            if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3800, 2) {
                if !data.is_empty() {
                    state.mix_raw = data[0].clamp(0, 127);
                }
            }
        }

        Ok(Box::new(dev))
    }

    /// Asegura que el listener reactivo de ALSA esté corriendo en segundo plano.
    fn ensure_alsa_listener(&mut self) {
        if self.listener_thread.is_some() {
            return;
        }

        let dev_node = match find_control_dev_node(&self.card) {
            Some(node) => node,
            None => {
                tracing::debug!(card = %self.card, "No se encontró nodo /dev/snd/controlC* para suscripción reactiva");
                return;
            }
        };

        let stop = Arc::clone(&self.listener_stop);
        let sink_holder = Arc::clone(&self.hw_sink);
        let shared = Arc::clone(&self.shared);

        stop.store(false, Ordering::Relaxed);
        let handle = std::thread::Builder::new()
            .name("evo-alsa-listener".to_string())
            .spawn(move || {
                run_alsa_listener(dev_node, stop, sink_holder, shared);
            });

        match handle {
            Ok(h) => self.listener_thread = Some(h),
            Err(e) => tracing::error!("No se pudo crear hilo evo-alsa-listener: {e}"),
        }
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
            let (mut gains, mut phantoms, mut mutes, mut mix) = {
                let s = self.shared.lock().unwrap();
                (s.gain_steps, s.phantom, s.mutes, s.mix_raw)
            };
            let mut stale = false;

            for ch in 0..2 {
                std::thread::sleep(std::time::Duration::from_millis(3));
                match evo_raw_get_cur(file, (2 << 8) | ((ch + 1) as u16), 0x0B00, 2) {
                    Ok(data) => {
                        if data.len() >= 2 {
                            let raw = i16::from_le_bytes([data[0], data[1]]);
                            let db = (raw as f32) / 256.0;
                            gains[ch] = db_to_step(db);
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
                        phantoms[0] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0001, 0x3A00, 4) {
                    if !data.is_empty() {
                        phantoms[1] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0200, 0x3A00, 4) {
                    if !data.is_empty() {
                        mutes[0] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0201, 0x3A00, 4) {
                    if !data.is_empty() {
                        mutes[1] = data[0] == 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
                if let Ok(data) = evo_raw_get_cur(file, 0x0000, 0x3800, 2) {
                    if !data.is_empty() {
                        mix = data[0].clamp(0, 127);
                    }
                }

                let mut s = self.shared.lock().unwrap();
                s.gain_steps = gains;
                s.phantom = phantoms;
                s.mutes = mutes;
                s.mix_raw = mix;
            }
        } else {
            // Modo fallback ALSA básico: actualizamos pasos de ganancia desde ALSA
            if let Ok(steps) = query_mic_volume(&self.card) {
                let mut s = self.shared.lock().unwrap();
                for (ch, &step) in steps.iter().enumerate().take(2) {
                    if s.mutes[ch] {
                        continue;
                    }
                    s.gain_steps[ch] = step;
                }
                if s.gain_steps.len() > 2 {
                    s.gain_steps[2] = steps[2];
                    s.gain_steps[3] = steps[3];
                }
            }
        }

        // Actualizamos volumen de salida desde ALSA numid=4
        if let Ok(vol) = query_playback_volume(&self.card) {
            let mut s = self.shared.lock().unwrap();
            s.volume_step = vol;
        }

        let s = self.shared.lock().unwrap();
        let mut state = DeviceState::initial(&self.profile);
        state.channels[0].gain_db = step_to_db(s.gain_steps[0]);
        state.channels[1].gain_db = step_to_db(s.gain_steps[1]);
        state.channels[0].muted = s.mutes[0];
        state.channels[1].muted = s.mutes[1];
        state.channels[0].phantom = s.phantom[0];
        state.channels[1].phantom = s.phantom[1];
        state.loopback = self.loopback;
        state.smartgain = self.smartgain;

        let vol_db = (s.volume_step as f32) * 0.5 - 127.0;
        let pan = ((s.mix_raw as f32) / 127.0) * 2.0 - 1.0;
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
                let (active_steps, mutes) = {
                    let mut s = self.shared.lock().unwrap();
                    s.gain_steps[ch as usize] = step;
                    (s.gain_steps, s.mutes)
                };

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
                    let _ = write_mic_volume(&self.card, &active_steps);
                } else {
                    // En modo ALSA básico, si está muteado mantenemos 0 en el control ALSA
                    let mut steps = active_steps;
                    if mutes[0] {
                        steps[0] = 0;
                    }
                    if mutes[1] {
                        steps[1] = 0;
                    }
                    let _ = write_mic_volume(&self.card, &steps);
                }

                Ok(vec![DeviceEvent::GainChanged { ch, db }])
            }

            DeviceCommand::SetMute { ch, on } => {
                if ch as usize >= 2 {
                    return Err(DeviceError::Rejected(format!("canal {ch} fuera de rango")));
                }
                let (active_steps, mutes) = {
                    let mut s = self.shared.lock().unwrap();
                    s.mutes[ch as usize] = on;
                    (s.gain_steps, s.mutes)
                };

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
                    let mut steps = active_steps;
                    if mutes[0] {
                        steps[0] = 0;
                    }
                    if mutes[1] {
                        steps[1] = 0;
                    }
                    let _ = write_mic_volume(&self.card, &steps);
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
                    {
                        let mut s = self.shared.lock().unwrap();
                        s.phantom[ch as usize] = on;
                    }
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
                {
                    let mut s = self.shared.lock().unwrap();
                    s.volume_step = step;
                }
                let _ = write_playback_volume(&self.card, step);

                // 2. Mix entre PC y placa (Extension Unit 56 CS=0 en /dev/evo4):
                // pan en [-1.0, 1.0] -> 0 = 100% entrada/placa, 127 = 100% PC/DAW, 64 = 50/50 balance
                let raw_mix = (((pan + 1.0) / 2.0) * 127.0).round().clamp(0.0, 127.0) as u8;
                {
                    let mut s = self.shared.lock().unwrap();
                    s.mix_raw = raw_mix;
                }

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
                self.smartgain = SmartGainPhase::Idle;
                Ok(vec![DeviceEvent::SmartGainChanged {
                    phase: SmartGainPhase::Idle,
                }])
            }

            other => Err(DeviceError::Unsupported(other.op_name())),
        }
    }

    fn set_hw_event_sink(&mut self, sink: HwEventCallback) {
        *self.hw_sink.lock().unwrap() = Some(sink);
        self.ensure_alsa_listener();
    }

    fn close(&mut self) -> Result<(), DeviceError> {
        self.listener_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
        self.raw_dev = None;
        Ok(())
    }
}

impl Drop for Evo4HwDevice {
    fn drop(&mut self) {
        let _ = self.close();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_step_conversion() {
        assert_eq!(db_to_step(-8.0), 0);
        assert_eq!(db_to_step(0.0), 16);
        assert_eq!(db_to_step(50.0), 116);
        assert_eq!(db_to_step(-20.0), 0); // clamp min
        assert_eq!(db_to_step(100.0), 116); // clamp max

        assert!((step_to_db(0) - (-8.0)).abs() < 1e-4);
        assert!((step_to_db(16) - 0.0).abs() < 1e-4);
        assert!((step_to_db(116) - 50.0).abs() < 1e-4);
    }

    #[test]
    fn test_find_control_dev_node() {
        assert!(find_control_dev_node("non_existent_card_xyz123").is_none());
        // Numeric card index resolution if card 0 exists
        if Path::new("/dev/snd/controlC0").exists() {
            assert_eq!(
                find_control_dev_node("0"),
                Some("/dev/snd/controlC0".to_string())
            );
            assert_eq!(
                find_control_dev_node("hw:0"),
                Some("/dev/snd/controlC0".to_string())
            );
        }
    }

    #[test]
    fn test_evo_shared_state_default() {
        let state = EvoSharedState::default();
        assert_eq!(state.gain_steps, [0, 0, 0, 0]);
        assert_eq!(state.volume_step, 0);
        assert_eq!(state.mutes, [false, false]);
        assert_eq!(state.phantom, [false, false]);
        assert_eq!(state.mix_raw, 0);
    }
}

