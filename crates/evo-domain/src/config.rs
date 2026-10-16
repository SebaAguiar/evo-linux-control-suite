//! Configuración de la suite que se persiste junto al estado de dispositivo.
//!
//! El SAD §8.5 mantiene **un único documento** `config.toml` con los ajustes de
//! la aplicación (`[server]`, `[ui]`) y el último estado comandado del
//! dispositivo. Aquí vive la parte pura (sin I/O) de esos ajustes; el adaptador
//! de archivo los escribe y los lee (SAD §3.1).
//!
//! Solo se modela lo que **alguien lee**: el daemon consume `[server]` a partir de
//! este bloque. `[ui]` y `[[presets]]` llegarán con sus consumidores (la UI y la
//! biblioteca de presets) en lugar de inventarse campos vacíos ahora.

use serde::{Deserialize, Serialize};

/// Ajustes del servidor de control remoto (SAD §8.4, §8.5).
///
/// El servidor remoto es **opt-in** (SAD §8.4): por defecto está desactivado y
/// solo se enlaza loopback. Cuando se activa, escucha en la IP de la interfaz de
/// LAN indicada, además de loopback; nunca en `0.0.0.0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// ¿Enlazar la interfaz de LAN además de loopback?
    pub enabled: bool,
    /// Interfaz de red en la que escuchar (`"wlan0"`, `"eth0"`, …).
    ///
    /// `None` significa "ninguna": con `enabled = true` pero sin interfaz, no hay
    /// dónde enlazar y el daemon se queda en loopback y lo registra. Se omite al
    /// serializar porque TOML no tiene un valor nulo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// Puerto del control remoto (loopback y LAN comparten puerto).
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface: None,
            port: 8080,
        }
    }
}
