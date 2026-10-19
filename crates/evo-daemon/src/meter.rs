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
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    // 2. Intentar con pactl
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
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Lanza el subproceso de captura de audio crudo en modo pasivo.
fn spawn_capture_child(target: Option<&str>, channels: u8) -> Result<Child, MeterError> {
    // 1. Intentar pw-record (nativo PipeWire, no intrusivo)
    if Command::new("pw-record").arg("--version").output().is_ok() {
        let mut cmd = Command::new("pw-record");
        cmd.args([
            "-a",
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

/// Fuente de medidores derivada de la captura PCM real en segundo plano.
pub struct PcmMeterSource {
    target: Option<String>,
    channels: u8,
    frame: Arc<Mutex<MeterFrame>>,
    stop: Arc<AtomicBool>,
    child: Option<Child>,
    thread: Option<JoinHandle<()>>,
}

impl PcmMeterSource {
    pub fn new() -> Self {
        Self {
            target: detect_evo_node(),
            channels: 2,
            frame: Arc::new(Mutex::new(MeterFrame::silent(0, 2))),
            stop: Arc::new(AtomicBool::new(false)),
            child: None,
            thread: None,
        }
    }

    pub fn with_target(target: impl Into<String>) -> Self {
        Self {
            target: Some(target.into()),
            channels: 2,
            frame: Arc::new(Mutex::new(MeterFrame::silent(0, 2))),
            stop: Arc::new(AtomicBool::new(false)),
            child: None,
            thread: None,
        }
    }
}

impl Default for PcmMeterSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterSource for PcmMeterSource {
    fn start(&mut self, profile: &DeviceProfile) -> Result<(), MeterError> {
        if self.thread.is_some() {
            return Ok(());
        }

        self.channels = profile.inputs;
        *self.frame.lock().unwrap() = MeterFrame::silent(0, self.channels);

        let mut child = spawn_capture_child(self.target.as_deref(), self.channels)?;
        let mut stdout = match child.stdout.take() {
            Some(out) => out,
            None => {
                let _ = child.kill();
                return Err(MeterError::Backend("stdout no accesible".into()));
            }
        };

        self.stop.store(false, Ordering::Relaxed);
        let stop = Arc::clone(&self.stop);
        let shared_frame = Arc::clone(&self.frame);
        let channels = self.channels;

        let thread = std::thread::Builder::new()
            .name("evo-pcm-meter".to_string())
            .spawn(move || {
                let bytes_per_frame = (channels as usize) * 2;
                // Ventana de cómputo de 10 ms (480 muestras a 48 kHz)
                let chunk_frames = 480;
                let chunk_bytes = chunk_frames * bytes_per_frame;
                let mut buf = vec![0u8; chunk_bytes];
                let mut current_peaks_db = vec![-128.0f32; channels as usize];
                let mut seq: u16 = 0;

                while !stop.load(Ordering::Relaxed) {
                    if stdout.read_exact(&mut buf).is_err() {
                        break;
                    }

                    // Encontrar el valor pico absoluto en este bloque de 10 ms
                    let mut chunk_max = vec![0i16; channels as usize];
                    for frame_slice in buf.chunks_exact(bytes_per_frame) {
                        for (ch, sample_bytes) in frame_slice.chunks_exact(2).enumerate() {
                            if ch < channels as usize {
                                let val = i16::from_le_bytes([sample_bytes[0], sample_bytes[1]]);
                                let abs_val = if val == i16::MIN { i16::MAX } else { val.abs() };
                                if abs_val > chunk_max[ch] {
                                    chunk_max[ch] = abs_val;
                                }
                            }
                        }
                    }

                    // Conversión a dBFS y balística (ataque inmediato, caída exponencial de ~1.5 dB por 10 ms)
                    let decay_step = 1.5;
                    for ch in 0..(channels as usize) {
                        let max_val = chunk_max[ch];
                        let db = if max_val <= 10 {
                            -128.0
                        } else {
                            20.0 * ((max_val as f32) / 32767.0).log10()
                        };

                        if db > current_peaks_db[ch] {
                            current_peaks_db[ch] = db;
                        } else {
                            current_peaks_db[ch] =
                                (current_peaks_db[ch] - decay_step).max(db).max(-128.0);
                        }
                    }

                    seq = seq.wrapping_add(1);
                    let new_frame = MeterFrame::from_db_fs(seq, &current_peaks_db);
                    if let Ok(mut lock) = shared_frame.lock() {
                        *lock = new_frame;
                    }
                }

                tracing::debug!("Hilo de medición evo-pcm-meter finalizado");
            })
            .map_err(|e| MeterError::Backend(e.to_string()))?;

        self.child = Some(child);
        self.thread = Some(thread);
        Ok(())
    }

    fn latest(&self) -> MeterFrame {
        *self.frame.lock().unwrap()
    }

    fn stop(&mut self) -> Result<(), MeterError> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(thread) = self.thread.take() {
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
        assert_eq!(frame.channels(), 2);
        assert_eq!(frame.peak_raw(0), Some(i8::MIN));
        assert_eq!(frame.peak_raw(1), Some(i8::MIN));
    }
}
