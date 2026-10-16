//! Identidad de los modelos de hardware soportados.

use serde::{Deserialize, Serialize};

/// Identificador de modelo de la familia Audient EVO.
///
/// `ModelId` es la **clave de estrategia** del `DriverRegistry` (SAD §1.5):
/// añadir un modelo = añadir un módulo driver y una entrada de registro, sin
/// tocar el núcleo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelId {
    /// Audient EVO 4 — 2 in / 2 out. Objetivo de la Fase 1.
    Evo4,
    /// Audient EVO 8 — 4 in / 4 out. Fase 3.
    Evo8,
    /// Audient EVO 16 — 8 in / 8 out. Fase 3.
    Evo16,
}

impl ModelId {
    /// Vendor ID de Audient Ltd. (SAD §4.4, regla udev `idVendor`).
    pub const VENDOR_ID: u16 = 0x2708;

    /// Código estable usado en persistencia, TOML y el contrato IPC
    /// (`'evo4' | 'evo8' | 'evo16'`, SAD §3.2).
    pub const fn code(self) -> &'static str {
        match self {
            ModelId::Evo4 => "evo4",
            ModelId::Evo8 => "evo8",
            ModelId::Evo16 => "evo16",
        }
    }

    /// Nombre para mostrar en la UI.
    pub const fn display_name(self) -> &'static str {
        match self {
            ModelId::Evo4 => "Audient EVO 4",
            ModelId::Evo8 => "Audient EVO 8",
            ModelId::Evo16 => "Audient EVO 16",
        }
    }

    /// Product ID conocida, si se conoce.
    ///
    /// Devuelve `None` a propósito: los PID de cada modelo **no están
    /// confirmados** todavía y son objeto de la validación de Fase 1
    /// (SAD §7.1, §8.7). El `DriverRegistry` debe resolver por
    /// `(vendor_id, product_id)` una vez verificados con `lsusb` sobre hardware
    /// real, no por suposición.
    pub const fn product_id(self) -> Option<u16> {
        let _ = self;
        None
    }

    /// Todos los modelos conocidos, en orden ascendente de tamaño.
    pub const ALL: [ModelId; 3] = [ModelId::Evo4, ModelId::Evo8, ModelId::Evo16];
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}
