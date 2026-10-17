//! Perfil de dispositivo y matriz de capacidades (SAD §1.5).

use crate::model::ModelId;
use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    /// Capacidades de un modelo, consultadas por la UI y el servidor remoto
    /// para **renderizar o rechazar** controles sin condicionales por modelo
    /// (SAD §1.5, "matriz de capacidades").
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Capabilities: u32 {
        /// Phantom power (+48 V) conmutable por canal.
        const PHANTOM_PER_CHANNEL = 1 << 0;
        /// Asistente SmartGain (ganancia automática).
        const SMARTGAIN           = 1 << 1;
        /// Loop-back (mezcla de reproducción hacia el stream de captura).
        const LOOPBACK            = 1 << 2;
        /// Entradas ADAT (expansión óptica).
        const ADAT                = 1 << 3;
        /// Mezclador de monitorización con level/pan por bus.
        const MONITOR_MIX         = 1 << 4;
        /// Mute por canal.
        const MUTE_PER_CHANNEL    = 1 << 5;
        /// Pad / atenuación de entrada.
        const INPUT_PAD           = 1 << 6;
        /// Alimentación de línea conmutable (instrumento/DI) por canal.
        const INSTRUMENT_PER_CHANNEL = 1 << 7;
    }
}

/// Rango de ganancia en dB de los preamplificadores de un modelo.
///
/// Se modela como struct con dos campos en lugar de `RangeInclusive<f32>`
/// (como aparece en el pseudocódigo del SAD) porque un rango inclusivo se
/// serializa de forma incómoda (`{"start":…,"end":…}`) y el contrato IPC debe ser
/// estable y legible.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GainRange {
    pub min_db: f32,
    pub max_db: f32,
}

impl GainRange {
    /// Rango provisional usado en ausencia de datos verificados de hardware.
    ///
    /// **A validar en Fase 1** (SAD §8.7): los límites reales del preamplificador
    /// del EVO deben confirmarse contra la unidad física antes de fijarlos.
    pub const PROVISIONAL: GainRange = GainRange {
        min_db: 0.0,
        max_db: 56.0,
    };

    pub const fn new(min_db: f32, max_db: f32) -> Self {
        Self { min_db, max_db }
    }

    pub fn contains(&self, db: f32) -> bool {
        db >= self.min_db && db <= self.max_db
    }

    /// Ajusta `db` al rango. Valores no finitos se devuelven intactos: es
    /// responsabilidad de la [`crate::safety::SafetyPolicy`] rechazarlos.
    pub fn clamp(&self, db: f32) -> f32 {
        if !db.is_finite() {
            return db;
        }
        db.clamp(self.min_db, self.max_db)
    }

    pub const fn as_range_inclusive(&self) -> std::ops::RangeInclusive<f32> {
        self.min_db..=self.max_db
    }
}

/// Descripción estática de un modelo: canales, capacidades y rangos.
///
/// Es lo que la UI recibe en el `snapshot` para construir sus controles
/// dinámicamente (SAD §1.5, §2.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub model: ModelId,
    /// Canales de entrada (mono, indexados desde 0).
    pub inputs: u8,
    /// Salidas / buses de monitorización.
    pub outputs: u8,
    pub caps: Capabilities,
    pub gain_range_db: GainRange,
    /// Duración máxima del ciclo SmartGain antes de abortar por seguridad.
    pub smartgain_timeout_ms: u32,
}

impl DeviceProfile {
    /// Perfil del EVO 4: 2 in / 2 out.
    pub fn evo4() -> Self {
        Self {
            model: ModelId::Evo4,
            inputs: 2,
            outputs: 2,
            caps: Capabilities::PHANTOM_PER_CHANNEL
                | Capabilities::SMARTGAIN
                | Capabilities::LOOPBACK
                | Capabilities::MONITOR_MIX
                | Capabilities::MUTE_PER_CHANNEL,
            gain_range_db: GainRange::PROVISIONAL,
            smartgain_timeout_ms: 15_000,
        }
    }

    /// Perfil del EVO 8: 4 in / 4 out.
    pub fn evo8() -> Self {
        Self {
            inputs: 4,
            outputs: 4,
            ..Self::evo4()
        }
        .with_model(ModelId::Evo8)
    }

    /// Perfil del EVO 16: 8 in / 8 out, con ADAT y mezclador de monitor.
    pub fn evo16() -> Self {
        Self {
            model: ModelId::Evo16,
            inputs: 8,
            outputs: 8,
            caps: Capabilities::PHANTOM_PER_CHANNEL
                | Capabilities::SMARTGAIN
                | Capabilities::LOOPBACK
                | Capabilities::ADAT
                | Capabilities::MONITOR_MIX
                | Capabilities::MUTE_PER_CHANNEL
                | Capabilities::INPUT_PAD
                | Capabilities::INSTRUMENT_PER_CHANNEL,
            gain_range_db: GainRange::PROVISIONAL,
            smartgain_timeout_ms: 15_000,
        }
    }

    /// Perfil por defecto de un modelo.
    pub fn for_model(model: ModelId) -> Self {
        match model {
            ModelId::Evo4 => Self::evo4(),
            ModelId::Evo8 => Self::evo8(),
            ModelId::Evo16 => Self::evo16(),
        }
    }

    fn with_model(mut self, model: ModelId) -> Self {
        self.model = model;
        self
    }

    /// ¿El canal de entrada existe en este modelo?
    pub fn has_input(&self, ch: u8) -> bool {
        ch < self.inputs
    }

    /// ¿La salida/bus existe en este modelo?
    pub fn has_output(&self, out: u8) -> bool {
        out < self.outputs
    }

    pub fn supports(&self, cap: Capabilities) -> bool {
        self.caps.contains(cap)
    }
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self::evo4()
    }
}
