//! Ruteo directo de audio por software (Software Direct Monitoring / Loopback).
//!
//! Permite puentear entradas físicas (como la entrada de instrumento JFET en Canal 1)
//! directamente a la salida de auriculares con latencia ultra-baja (< 2 ms)
//! a través de PipeWire (`pw-link`) o PulseAudio (`pactl module-loopback`),
//! evitando el circuito de mezcla de hardware de la placa (SAD §8.4)
//! y preservando la máxima fidelidad de audio digital estéreo sin pérdida de señal.

use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Estado público del ruteo directo por software.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoopbackStatus {
    /// Indica si el ruteo directo está actualmente activo.
    pub active: bool,
    /// Canales de entrada ruteados (0 = Canal 1/Instrumento, 1 = Canal 2).
    pub channels: Vec<u8>,
    /// Backend utilizado: `"pipewire"`, `"pulseaudio"` o `"none"`.
    pub backend: String,
    /// Nombre de la fuente de captura conectada.
    pub source: Option<String>,
    /// Nombre del destino de salida conectado.
    pub sink: Option<String>,
    /// Error si ocurrió alguno durante el intento de enlace.
    pub error: Option<String>,
}

struct LoopbackInner {
    active: bool,
    channels: Vec<u8>,
    backend: String,
    pw_links: Vec<(String, String)>,
    pulse_module: Option<u32>,
    source_name: Option<String>,
    sink_name: Option<String>,
    error: Option<String>,
}

/// Gestor del ruteo por software con soporte dual PipeWire y PulseAudio.
#[derive(Clone)]
pub struct LoopbackManager {
    inner: Arc<RwLock<LoopbackInner>>,
}

impl Default for LoopbackManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopbackManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(LoopbackInner {
                active: false,
                channels: vec![0],
                backend: "none".to_string(),
                pw_links: Vec::new(),
                pulse_module: None,
                source_name: None,
                sink_name: None,
                error: None,
            })),
        }
    }

    /// Devuelve el estado actual del ruteo.
    pub async fn status(&self) -> LoopbackStatus {
        let state = self.inner.read().await;
        LoopbackStatus {
            active: state.active,
            channels: state.channels.clone(),
            backend: state.backend.clone(),
            source: state.source_name.clone(),
            sink: state.sink_name.clone(),
            error: state.error.clone(),
        }
    }

    /// Activa el ruteo directo para los canales especificados (por defecto Canal 1 / Instrumento).
    pub async fn enable(&self, channels: Vec<u8>) -> Result<LoopbackStatus, String> {
        let mut state = self.inner.write().await;
        self.stop_locked(&mut state);

        let target_channels = if channels.is_empty() {
            vec![0]
        } else {
            channels
        };
        state.channels = target_channels.clone();

        // 1. Intentar con PipeWire pw-link (nativo, latencia mínima absoluta)
        match Self::try_pw_link(&target_channels) {
            Ok((links, src, snk)) => {
                tracing::info!(
                    source = %src,
                    sink = %snk,
                    pairs = links.len(),
                    "ruteo directo activado exitosamente mediante PipeWire pw-link"
                );
                state.active = true;
                state.backend = "pipewire".to_string();
                state.pw_links = links;
                state.source_name = Some(src);
                state.sink_name = Some(snk);
                state.error = None;

                return Ok(LoopbackStatus {
                    active: true,
                    channels: state.channels.clone(),
                    backend: state.backend.clone(),
                    source: state.source_name.clone(),
                    sink: state.sink_name.clone(),
                    error: None,
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "pw-link fallo o no disponible; probando fallback pactl module-loopback");
            }
        }

        // 2. Fallback a pactl load-module module-loopback
        match Self::try_pactl_loopback() {
            Ok((mod_id, src, snk)) => {
                tracing::info!(
                    module_id = mod_id,
                    source = %src,
                    sink = %snk,
                    "ruteo directo activado exitosamente mediante PulseAudio module-loopback"
                );
                state.active = true;
                state.backend = "pulseaudio".to_string();
                state.pulse_module = Some(mod_id);
                state.source_name = Some(src);
                state.sink_name = Some(snk);
                state.error = None;

                Ok(LoopbackStatus {
                    active: true,
                    channels: state.channels.clone(),
                    backend: state.backend.clone(),
                    source: state.source_name.clone(),
                    sink: state.sink_name.clone(),
                    error: None,
                })
            }
            Err(e) => {
                let err_msg = format!("no se pudo activar el ruteo directo: {e}");
                tracing::error!(error = %err_msg);
                state.active = false;
                state.error = Some(err_msg.clone());
                Err(err_msg)
            }
        }
    }

    /// Desactiva cualquier ruteo o loopback activo.
    pub async fn disable(&self) -> LoopbackStatus {
        let mut state = self.inner.write().await;
        self.stop_locked(&mut state);
        state.active = false;
        state.error = None;
        LoopbackStatus {
            active: false,
            channels: state.channels.clone(),
            backend: "none".to_string(),
            source: None,
            sink: None,
            error: None,
        }
    }

    fn stop_locked(&self, state: &mut LoopbackInner) {
        // Desconectar enlaces pw-link
        for (out_port, in_port) in state.pw_links.drain(..) {
            let _ = Command::new("pw-link")
                .args(["-d", &out_port, &in_port])
                .output();
        }

        // Descargar módulo pulseaudio si existe
        if let Some(mod_id) = state.pulse_module.take() {
            let _ = Command::new("pactl")
                .args(["unload-module", &mod_id.to_string()])
                .output();
        }

        state.backend = "none".to_string();
        state.source_name = None;
        state.sink_name = None;
    }

    /// Lista puertos de PipeWire vía `pw-link`.
    fn get_pw_ports(arg: &str) -> Result<Vec<String>, String> {
        let output = Command::new("pw-link")
            .arg(arg)
            .output()
            .map_err(|e| format!("pw-link no disponible: {e}"))?;

        if !output.status.success() {
            return Err("pw-link termino con error".to_string());
        }

        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with("Midi")
                    && !l.contains("Midi-Bridge")
                    && !l.contains("bluez_midi")
            })
            .collect())
    }

    /// Intenta conectar puertos de captura y reproducción vía `pw-link`.
    fn try_pw_link(
        channels: &[u8],
    ) -> Result<(Vec<(String, String)>, String, String), String> {
        let all_outputs = Self::get_pw_ports("-o")?;
        let all_inputs = Self::get_pw_ports("-i")?;

        let capture_ports: Vec<String> = all_outputs
            .into_iter()
            .filter(|p| p.contains("capture"))
            .collect();

        let playback_ports: Vec<String> = all_inputs
            .into_iter()
            .filter(|p| p.contains("playback"))
            .collect();

        if capture_ports.is_empty() {
            return Err("no se encontraron puertos de captura de audio".to_string());
        }
        if playback_ports.is_empty() {
            return Err("no se encontraron puertos de reproduccion de audio".to_string());
        }

        // 1. Filtrar puertos de la interfaz EVO si existen
        let evo_caps: Vec<String> = capture_ports
            .iter()
            .filter(|p| {
                let l = p.to_lowercase();
                l.contains("evo4") || l.contains("audient")
            })
            .cloned()
            .collect();

        let target_caps = if !evo_caps.is_empty() {
            evo_caps
        } else {
            capture_ports
        };

        let evo_plays: Vec<String> = playback_ports
            .iter()
            .filter(|p| {
                let l = p.to_lowercase();
                (l.contains("evo4") || l.contains("audient")) && !l.contains(".split")
            })
            .cloned()
            .collect();

        let target_plays = if !evo_plays.is_empty() {
            evo_plays
        } else {
            playback_ports
        };

        let mut created_links = Vec::new();
        let snk_name = target_plays
            .first()
            .map(|p| p.split(':').next().unwrap_or(p).to_string())
            .unwrap_or_else(|| "default_sink".to_string());

        let src_name = target_caps
            .first()
            .map(|p| p.split(':').next().unwrap_or(p).to_string())
            .unwrap_or_else(|| "default_source".to_string());

        // Conectar cada canal pedido
        for &ch in channels {
            let cap_port = if (ch as usize) < target_caps.len() {
                &target_caps[ch as usize]
            } else if let Some(first) = target_caps.first() {
                first
            } else {
                continue;
            };

            // Para que el instrumento suene en estéreo centrado en los auriculares,
            // conectamos a la salida izquierda y derecha si ambas están presentes.
            let play_left = target_plays.iter().find(|p| p.ends_with("_FL") || p.ends_with("_1"));
            let play_right = target_plays.iter().find(|p| p.ends_with("_FR") || p.ends_with("_2"));

            let destinations = match (play_left, play_right) {
                (Some(l), Some(r)) => vec![l.clone(), r.clone()],
                (Some(l), None) => vec![l.clone()],
                (None, Some(r)) => vec![r.clone()],
                (None, None) => target_plays.iter().take(2).cloned().collect(),
            };

            for dst in destinations {
                let res = Command::new("pw-link")
                    .args(["-L", cap_port, &dst])
                    .output();
                if let Ok(out) = res {
                    if out.status.success() {
                        created_links.push((cap_port.clone(), dst));
                    }
                }
            }
        }

        if created_links.is_empty() {
            return Err("pw-link no pudo crear ninguno de los enlaces pedidos".to_string());
        }

        Ok((created_links, src_name, snk_name))
    }

    /// Fallback mediante `pactl load-module module-loopback`.
    fn try_pactl_loopback() -> Result<(u32, String, String), String> {
        let src = Self::detect_pactl_source().unwrap_or_else(|| "@DEFAULT_SOURCE@".to_string());
        let snk = Self::detect_pactl_sink().unwrap_or_else(|| "@DEFAULT_SINK@".to_string());

        let output = Command::new("pactl")
            .args([
                "load-module",
                "module-loopback",
                &format!("source={src}"),
                &format!("sink={snk}"),
                "latency_msec=1",
            ])
            .output()
            .map_err(|e| format!("fallo al ejecutar pactl: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("pactl error: {stderr}"));
        }

        let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let mod_id: u32 = out
            .parse()
            .map_err(|_| format!("ID de módulo no numérico devuelto por pactl: {out}"))?;

        Ok((mod_id, src, snk))
    }

    fn detect_pactl_source() -> Option<String> {
        if let Ok(output) = Command::new("pactl").args(["list", "sources", "short"]).output() {
            if output.status.success() {
                let out = String::from_utf8_lossy(&output.stdout);
                for line in out.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let name = parts[1];
                        let l = name.to_lowercase();
                        if (l.contains("evo4") || l.contains("audient")) && !l.contains(".monitor") {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    fn detect_pactl_sink() -> Option<String> {
        if let Ok(output) = Command::new("pactl").args(["list", "sinks", "short"]).output() {
            if output.status.success() {
                let out = String::from_utf8_lossy(&output.stdout);
                for line in out.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let name = parts[1];
                        let l = name.to_lowercase();
                        if (l.contains("evo4") || l.contains("audient")) && !l.contains(".split") {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_initial_state_is_inactive() {
        let mgr = LoopbackManager::new();
        let status = mgr.status().await;
        assert!(!status.active);
        assert_eq!(status.backend, "none");
        assert!(status.source.is_none());
        assert!(status.sink.is_none());
    }

    #[tokio::test]
    async fn loopback_disable_returns_inactive() {
        let mgr = LoopbackManager::new();
        let status = mgr.disable().await;
        assert!(!status.active);
        assert_eq!(status.backend, "none");
    }
}
