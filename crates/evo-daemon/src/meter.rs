//! Adaptador de medidores de audio en tiempo real derivado del flujo PCM (SAD §8.2).
//!
//! Captura los canales de audio desde PipeWire/ALSA de forma pasiva (sin alterar
//! la latencia ni bloquear el hilo de comandos) y computa el nivel de pico en dBFS
//! por canal con balística de caída suave para el vúmetro.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use evo_domain::{DeviceProfile, MeterError, MeterFrame};
use evo_ports::MeterSource;

/// Detecta el nombre de nodo de entrada en PipeWire/PulseAudio correspondiente a la EVO.
pub fn detect_evo_node() -> Option<String> {
    let mut candidates = Vec::new();

    // 1. Intentar con pw-cli
    if let Ok(output) = Command::new("pw-cli")
        .args(["list-objects", "Node"])
        .output()
    {
        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            for line in out.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("node.name = ") {
                    let name = trimmed
                        .trim_start_matches("node.name = ")
                        .trim_matches('"');
                    let lower = name.to_lowercase();
                    if (lower.contains("evo4") || lower.contains("audient"))
                        && (lower.contains("input") || lower.contains("source"))
                    {
                        candidates.push(name.to_string());
                    }
                }
            }
        }
    }

    // 2. Intentar con pactl
    if candidates.is_empty() {
        if let Ok(output) = Command::new("pactl")
            .args(["list", "sources", "short"])
            .output()
        {
            if output.status.success() {
                let out = String::from_utf8_lossy(&output.stdout);
                for line in out.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let name = parts[1];
                        let lower = name.to_lowercase();
                        if lower.contains("evo4") || lower.contains("audient") {
                            candidates.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // Prioridad 1: Nodo de hardware crudo (ej. alsa_input.hw_EVO4_0),
    // que contiene todos los canales físicos (mic1 y mic2) sin separación virtual UCM.
    if let Some(hw) = candidates.iter().find(|c| c.contains("hw_")) {
        return Some(hw.clone());
    }

    // Prioridad 2: Nodos de micrófono o fuentes principales que no sean .split ni Line (loopback)
    if let Some(node) = candidates.iter().find(|c| !c.contains(".split") && !c.contains("Line")) {
        return Some(node.clone());
    }

    candidates.into_iter().next()
}

/// Detecta el nombre de nodo de salida (sink / playback) en PipeWire correspondiente a la EVO.
pub fn detect_evo_output_node() -> Option<String> {
    // 0. Revisar primero si el sink por defecto actual del sistema pertenece a la EVO
    if let Ok(output) = Command::new("pactl").arg("get-default-sink").output() {
        if output.status.success() {
            let def = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let lower = def.to_lowercase();
            if (lower.contains("evo4") || lower.contains("audient")) && !lower.contains(".split") {
                return Some(def);
            }
        }
    }

    let mut candidates = Vec::new();

    if let Ok(output) = Command::new("pw-cli")
        .args(["list-objects", "Node"])
        .output()
    {
        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            for line in out.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("node.name = ") {
                    let name = trimmed
                        .trim_start_matches("node.name = ")
                        .trim_matches('"');
                    let lower = name.to_lowercase();
                    if (lower.contains("evo4") || lower.contains("audient"))
                        && (lower.contains("sink") || lower.contains("output"))
                    {
                        candidates.push(name.to_string());
                    }
                }
            }
        }
    }

    // 1. Preferir el sink de auriculares/line out principal (no split)
    if let Some(node) = candidates.iter().find(|c| c.contains("Headphones") && !c.contains(".split")) {
        return Some(node.clone());
    }

    // 2. Preferir nodo de hardware crudo (ej. alsa_output.hw_EVO4_0)
    if let Some(hw) = candidates.iter().find(|c| c.contains("hw_")) {
        return Some(hw.clone());
    }

    // 3. Cualquier sink sin split
    if let Some(node) = candidates.iter().find(|c| !c.contains(".split")) {
        return Some(node.clone());
    }

    candidates.into_iter().next()
}

/// Lanza el subproceso de captura de audio crudo en modo pasivo.
fn spawn_capture_child(target: Option<&str>, channels: u8) -> Result<Child, MeterError> {
    // 1. Intentar pw-record (nativo PipeWire, no intrusivo)
    if Command::new("pw-record").arg("--version").output().is_ok() {
        let mut cmd = Command::new("pw-record");
        cmd.args([
            "-a",
            "--latency=20ms",
            "--channels",
            &channels.to_string(),
            "--rate",
            "48000",
            "--format",
            "s16",
        ]);
        if let Some(t) = target {
            cmd.args(["--target", t]);
        }
        cmd.arg("-");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        if let Ok(child) = cmd.spawn() {
            tracing::debug!(target = ?target, "Captura de vúmetro iniciada con pw-record");
            return Ok(child);
        }
    }

    // 2. Fallback parec (servidor PulseAudio o compatibilidad PipeWire)
    if Command::new("parec").arg("--version").output().is_ok() {
        let mut cmd = Command::new("parec");
        cmd.args([
            "--raw",
            "--latency-msec=20",
            &format!("--channels={channels}"),
            "--rate=48000",
            "--format=s16le",
        ]);
        if let Some(t) = target {
            cmd.args(["-d", t]);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        if let Ok(child) = cmd.spawn() {
            tracing::debug!(target = ?target, "Captura de vúmetro iniciada con parec");
            return Ok(child);
        }
    }

    // 3. Fallback arecord (ALSA puro)
    let mut cmd = Command::new("arecord");
    let dev = target.unwrap_or("default");
    cmd.args([
        "-D",
        dev,
        "-c",
        &channels.to_string(),
        "-r",
        "48000",
        "-f",
        "S16_LE",
        "-t",
        "raw",
        "-",
    ]);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    cmd.spawn().map_err(|e| MeterError::Backend(e.to_string()))
}

/// Lanza el subproceso de captura de monitor de salida (playback) en modo pasivo.
fn spawn_monitor_capture_child(target: Option<&str>, channels: u8) -> Result<Child, MeterError> {
    // 1. Intentar parec (PipeWire-Pulse)
    // En PipeWire, un sink de salida se monitorea conectándose a su fuente .monitor.
    // parec con -d <sink>.monitor y baja latencia transmite en streaming sin bloquear el pipe.
    if Command::new("parec").arg("--version").output().is_ok() {
        let mut cmd = Command::new("parec");
        cmd.args([
            "--raw",
            "--latency-msec=20",
            &format!("--channels={channels}"),
            "--rate=48000",
            "--format=s16le",
        ]);
        if let Some(t) = target {
            let mon_target = if t.ends_with(".monitor") {
                t.to_string()
            } else {
                format!("{t}.monitor")
            };
            cmd.args(["-d", &mon_target]);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        if let Ok(child) = cmd.spawn() {
            tracing::debug!(target = ?target, "Captura de monitor de salida iniciada con parec");
            return Ok(child);
        }
    }

    // 2. Fallback
    spawn_capture_child(target, channels)
}

fn spawn_meter_stream_reader(
    mut stdout: std::process::ChildStdout,
    stop: Arc<AtomicBool>,
    shared_state: Arc<Mutex<([f32; 4], u16)>>,
    shared_frame: Arc<Mutex<MeterFrame>>,
    channel_offset: usize,
    num_channels: usize,
    thread_name: String,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let bytes_per_frame = num_channels * 2;
            let chunk_frames = 480;
            let chunk_bytes = chunk_frames * bytes_per_frame;
            let mut buf = vec![0u8; chunk_bytes];
            let decay_step = 1.5;

            while !stop.load(Ordering::Relaxed) {
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }

                let mut chunk_max = vec![0i16; num_channels];
                for frame_slice in buf.chunks_exact(bytes_per_frame) {
                    for (ch, sample_bytes) in frame_slice.chunks_exact(2).enumerate() {
                        if ch < num_channels {
                            let val = i16::from_le_bytes([sample_bytes[0], sample_bytes[1]]);
                            let abs_val = if val == i16::MIN { i16::MAX } else { val.abs() };
                            if abs_val > chunk_max[ch] {
                                chunk_max[ch] = abs_val;
                            }
                        }
                    }
                }

                if let Ok(mut state) = shared_state.lock() {
                    for ch in 0..num_channels {
                        let idx = channel_offset + ch;
                        if idx < 4 {
                            let max_val = chunk_max[ch];
                            let db = if max_val <= 10 {
                                -128.0
                            } else {
                                20.0 * ((max_val as f32) / 32767.0).log10()
                            };

                            let current = state.0[idx];
                            if db > current {
                                state.0[idx] = db;
                            } else {
                                state.0[idx] = (current - decay_step).max(db).max(-128.0);
                            }
                        }
                    }
                    state.1 = state.1.wrapping_add(1);
                    let seq = state.1;
                    let frame = MeterFrame::from_db_fs(seq, &state.0);
                    if let Ok(mut f_lock) = shared_frame.lock() {
                        *f_lock = frame;
                    }
                }
            }
        })
        .unwrap()
}

/// Fuente de medidores derivada de la captura PCM real en segundo plano (entradas + salidas).
pub struct PcmMeterSource {
    input_target: Option<String>,
    output_target: Option<String>,
    frame: Arc<Mutex<MeterFrame>>,
    stop: Arc<AtomicBool>,
    children: Vec<Child>,
    threads: Vec<JoinHandle<()>>,
}

impl PcmMeterSource {
    pub fn new() -> Self {
        Self {
            input_target: detect_evo_node(),
            output_target: detect_evo_output_node(),
            frame: Arc::new(Mutex::new(MeterFrame::silent(0, 4))),
            stop: Arc::new(AtomicBool::new(false)),
            children: Vec::new(),
            threads: Vec::new(),
        }
    }

    pub fn with_target(target: impl Into<String>) -> Self {
        Self {
            input_target: Some(target.into()),
            output_target: detect_evo_output_node(),
            frame: Arc::new(Mutex::new(MeterFrame::silent(0, 4))),
            stop: Arc::new(AtomicBool::new(false)),
            children: Vec::new(),
            threads: Vec::new(),
        }
    }
}

impl Default for PcmMeterSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterSource for PcmMeterSource {
    fn start(&mut self, _profile: &DeviceProfile) -> Result<(), MeterError> {
        if !self.threads.is_empty() {
            return Ok(());
        }

        *self.frame.lock().unwrap() = MeterFrame::silent(0, 4);
        self.stop.store(false, Ordering::Relaxed);

        let shared_state = Arc::new(Mutex::new(([-128.0f32; 4], 0u16)));
        let mut started_any = false;

        // 1. Stream de entradas (Mic 1 & Mic 2 -> canales 0 y 1)
        if let Ok(mut child) = spawn_capture_child(self.input_target.as_deref(), 2) {
            if let Some(stdout) = child.stdout.take() {
                let thread = spawn_meter_stream_reader(
                    stdout,
                    Arc::clone(&self.stop),
                    Arc::clone(&shared_state),
                    Arc::clone(&self.frame),
                    0,
                    2,
                    "evo-in-meter".to_string(),
                );
                self.children.push(child);
                self.threads.push(thread);
                started_any = true;
            } else {
                let _ = child.kill();
            }
        }

        // 2. Stream de salidas (Playback Music L & R -> canales 2 y 3)
        if let Ok(mut child) = spawn_monitor_capture_child(self.output_target.as_deref(), 2) {
            if let Some(stdout) = child.stdout.take() {
                let thread = spawn_meter_stream_reader(
                    stdout,
                    Arc::clone(&self.stop),
                    Arc::clone(&shared_state),
                    Arc::clone(&self.frame),
                    2,
                    2,
                    "evo-out-meter".to_string(),
                );
                self.children.push(child);
                self.threads.push(thread);
                started_any = true;
            } else {
                let _ = child.kill();
            }
        }

        if !started_any {
            return Err(MeterError::Backend(
                "No se pudo iniciar ningún flujo de captura de audio".into(),
            ));
        }

        Ok(())
    }

    fn latest(&self) -> MeterFrame {
        *self.frame.lock().unwrap()
    }

    fn stop(&mut self) -> Result<(), MeterError> {
        self.stop.store(true, Ordering::Relaxed);
        for mut child in self.children.drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        Ok(())
    }
}

impl Drop for PcmMeterSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Envoltorio sobre [`evo_mock::MockMeterSource`] que avanza la fase automáticamente en cada lectura.
pub struct AutoMockMeterSource {
    inner: Mutex<evo_mock::MockMeterSource>,
}

impl AutoMockMeterSource {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(evo_mock::MockMeterSource::new()),
        }
    }
}

impl Default for AutoMockMeterSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterSource for AutoMockMeterSource {
    fn start(&mut self, profile: &DeviceProfile) -> Result<(), MeterError> {
        self.inner.lock().unwrap().start(profile)
    }

    fn latest(&self) -> MeterFrame {
        let mut mock = self.inner.lock().unwrap();
        mock.advance();
        mock.latest()
    }

    fn stop(&mut self) -> Result<(), MeterError> {
        self.inner.lock().unwrap().stop()
    }
}

/// Construye la fuente de medidores adecuada según si hay hardware físico o se está simulando.
pub fn create_meter_source(is_hw: bool, profile: &DeviceProfile) -> Box<dyn MeterSource> {
    if is_hw {
        let mut pcm = PcmMeterSource::new();
        if let Err(e) = pcm.start(profile) {
            tracing::warn!(
                "No se pudo iniciar captura PCM nativa para vúmetro ({e}); usando fallback simulado"
            );
            let mut mock = AutoMockMeterSource::new();
            let _ = mock.start(profile);
            Box::new(mock)
        } else {
            Box::new(pcm)
        }
    } else {
        let mut mock = AutoMockMeterSource::new();
        let _ = mock.start(profile);
        Box::new(mock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm_meter_source_initial_silence() {
        let source = PcmMeterSource::new();
        let frame = source.latest();
        assert_eq!(frame.channels(), 4);
        assert_eq!(frame.peak_raw(0), Some(i8::MIN));
        assert_eq!(frame.peak_raw(1), Some(i8::MIN));
        assert_eq!(frame.peak_raw(2), Some(i8::MIN));
        assert_eq!(frame.peak_raw(3), Some(i8::MIN));
    }
}
