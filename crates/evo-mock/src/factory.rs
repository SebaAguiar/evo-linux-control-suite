//! Fábrica de drivers simulados, para poblar un [`evo_ports::DriverRegistry`]
//! sin hardware.

use evo_domain::{DeviceError, DeviceProfile, ModelId};
use evo_ports::{DeviceId, DriverFactory, EvoDevice};

use crate::MockDevice;

/// Fábrica que abre [`MockDevice`]s del modelo indicado.
///
/// Permite que el composition root y los tests ejerciten el registro real de
/// drivers (resolución por `(VID, PID)`, selección por modelo) sin un EVO físico.
pub struct MockDriverFactory {
    model: ModelId,
    device_id: Option<DeviceId>,
}

impl MockDriverFactory {
    /// Fábrica con VID de Audient y **sin PID confirmado**, que es la situación
    /// real hasta la validación de Fase 1 (SAD §8.7).
    pub fn new(model: ModelId) -> Self {
        Self {
            model,
            device_id: None,
        }
    }

    /// Fábrica con una identidad USB conocida, para probar la resolución por
    /// `(VID, PID)`.
    pub fn with_device_id(model: ModelId, product_id: u16) -> Self {
        Self {
            model,
            device_id: Some(DeviceId::new(ModelId::VENDOR_ID, product_id)),
        }
    }
}

impl DriverFactory for MockDriverFactory {
    fn model(&self) -> ModelId {
        self.model
    }

    fn profile(&self) -> DeviceProfile {
        DeviceProfile::for_model(self.model)
    }

    fn device_id(&self) -> Option<DeviceId> {
        self.device_id
    }

    fn open(&self) -> Result<Box<dyn EvoDevice>, DeviceError> {
        Ok(Box::new(MockDevice::new(self.model)))
    }
}
