//! Estado por canal.

use serde::{Deserialize, Serialize};

/// Estado de un canal de entrada.
///
/// Los canales se modelan como colecciones indexadas dimensionadas por el
/// [`crate::profile::DeviceProfile`], nunca como campos fijos: es lo que permite
/// escalar de 2 a 8 canales sin cambiar el esquema (SAD §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChannelState {
    /// Ganancia comandada en dB.
    pub gain_db: f32,
    pub muted: bool,
    /// Phantom power (+48 V). `true` significa *último valor comandado*: el
    /// hardware no lo notifica (SAD §8.6), así que puede divergir si algo cambia
    /// por fuera de la aplicación.
    pub phantom: bool,
    /// Atenuación de entrada (pad), si el modelo la soporta.
    pub pad: bool,
    /// Entrada de instrumento (DI), si el modelo la soporta.
    pub instrument: bool,
}

impl ChannelState {
    /// Estado inicial de un canal.
    ///
    /// `phantom` arranca en `false` **siempre**: es la postura segura por
    /// defecto ante micrófonos de cinta (SAD §4.3, §8.5).
    pub const fn new(gain_db: f32) -> Self {
        Self {
            gain_db,
            muted: false,
            phantom: false,
            pad: false,
            instrument: false,
        }
    }
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new(0.0)
    }
}
