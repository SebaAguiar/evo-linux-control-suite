//! Errores tipados del dominio y de los puertos.
//!
//! Se separan por origen para que cada capa pueda manejarlos sin `match` sobre
//! variantes imposibles: el núcleo no debe saber de fallos de transferencia USB,
//! y el adaptador USB no necesita saber de reglas de seguridad.

use thiserror::Error;

/// Error de validación o de invariante del núcleo puro.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DomainError {
    #[error("canal {ch} fuera de rango (el perfil declara {channels} canales)")]
    ChannelOutOfRange { ch: u8, channels: u8 },
    #[error("salida {out} fuera de rango (el perfil declara {outputs} salidas)")]
    OutputOutOfRange { out: u8, outputs: u8 },
    #[error("valor no finito para {param}")]
    NotFinite { param: &'static str },
    #[error("parámetro fuera de rango: {param} = {value} (esperado {expected})")]
    OutOfRange {
        param: &'static str,
        value: f32,
        expected: &'static str,
    },
}

/// Rechazo de un comando por [`crate::safety::SafetyPolicy`].
///
/// El hardware y los micrófonos conectados son el activo más valioso: ante la
/// duda, se rechaza (SAD §4.3, atributo de calidad #1).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SafetyError {
    #[error("valor no finito para {param}")]
    NotFinite { param: &'static str },
    #[error("phantom bloqueado globalmente desde el escritorio")]
    PhantomLocked,
    #[error("cambio de phantom demasiado pronto: faltan {remaining_ms} ms de enfriamiento")]
    PhantomCooldown { remaining_ms: u64 },
    #[error("ya hay una ejecución de SmartGain en curso")]
    SmartGainBusy,
    #[error("el modelo no soporta esta función: {feature}")]
    Unsupported { feature: &'static str },
}

/// Error de un adaptador de dispositivo (USB, mock, …).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DeviceError {
    #[error("dispositivo no presente")]
    NotFound,
    #[error("dispositivo desconectado durante la operación")]
    Disconnected,
    #[error("fallo de transferencia: {0}")]
    Transfer(String),
    #[error("timeout de operación")]
    Timeout,
    #[error("el dispositivo rechazó el comando: {0}")]
    Rejected(String),
    #[error("operación no soportada por este dispositivo: {0}")]
    Unsupported(&'static str),
    #[error("error de backend: {0}")]
    Backend(String),
}

/// Error del puerto [`crate::meter::MeterFrame`] fuente de medidores.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum MeterError {
    #[error("PipeWire/ALSA no disponible")]
    BackendUnavailable,
    #[error("no se pudo mapear el nodo del dispositivo a canales del perfil")]
    ChannelMappingFailed,
    #[error("error de backend: {0}")]
    Backend(String),
}

/// Error del puerto de persistencia.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RepoError {
    #[error("no se pudo leer la configuración: {0}")]
    Read(String),
    #[error("no se pudo escribir la configuración: {0}")]
    Write(String),
    #[error("documento corrupto o de versión desconocida: {0}")]
    Corrupt(String),
}

/// Motivo por el que un comando fue negado, antes de llegar al hardware.
///
/// Unifica las dos causas posibles: violar un invariante del dominio (índice
/// inexistente) o violar la [`crate::safety::SafetyPolicy`]. Quien rechaza
/// comandos solo necesita `match` sobre este tipo.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CommandRejection {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Safety(#[from] SafetyError),
}
