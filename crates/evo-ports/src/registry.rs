//! Registro de drivers: `(VID, PID) → DriverFactory`.
//!
//! Materializa el patrón *Strategy + Registry* del SAD §1.5: añadir un modelo es
//! añadir un módulo y una entrada de registro, sin tocar el núcleo (principio
//! abierto/cerrado).

use crate::EvoDevice;
use evo_domain::{DeviceError, DeviceProfile, ModelId};

/// Identidad USB de un dispositivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId {
    pub vendor_id: u16,
    pub product_id: u16,
}

impl DeviceId {
    pub const fn new(vendor_id: u16, product_id: u16) -> Self {
        Self {
            vendor_id,
            product_id,
        }
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

/// Fábrica de dispositivos para un modelo.
///
/// Separar "describir" de "abrir" permite que el registro sirva a la UI (para
/// listar modelos soportados) sin tocar hardware.
pub trait DriverFactory: Send + Sync {
    /// Modelo que atiende esta fábrica.
    fn model(&self) -> ModelId;

    /// Perfil del modelo, disponible sin abrir el dispositivo.
    fn profile(&self) -> DeviceProfile;

    /// Identidad USB que reconoce.
    ///
    /// `None` cuando el Product ID todavía no está confirmado contra hardware
    /// real; en ese caso el registro no puede resolver por `(VID, PID)` y hay que
    /// usar [`DriverRegistry::by_model`] (SAD §8.7, criterio de Fase 1).
    fn device_id(&self) -> Option<DeviceId>;

    /// Abre una instancia del dispositivo.
    ///
    /// Es `&self` y no `&mut self` para que el registro pueda compartirse sin
    /// contienda; el estado mutable vive en la instancia devuelta.
    fn open(&self) -> Result<Box<dyn EvoDevice>, DeviceError>;
}

/// Registro de fábricas de drivers disponibles.
#[derive(Default)]
pub struct DriverRegistry {
    factories: Vec<Box<dyn DriverFactory>>,
}

impl DriverRegistry {
    pub const fn new() -> Self {
        Self {
            factories: Vec::new(),
        }
    }

    /// Registra una fábrica. Devuelve `&mut Self` para encadenar.
    pub fn register(&mut self, factory: Box<dyn DriverFactory>) -> &mut Self {
        self.factories.push(factory);
        self
    }

    /// Busca la fábrica que reconoce una identidad USB concreta.
    ///
    /// En Fase 1 esto puede no encontrar nada porque los PID no están
    /// confirmados; el arranque debe caer entonces a [`Self::by_model`] o al
    /// dispositivo simulado.
    pub fn by_ids(&self, id: DeviceId) -> Option<&dyn DriverFactory> {
        self.factories
            .iter()
            .find(|f| f.device_id() == Some(id))
            .map(AsRef::as_ref)
    }

    /// Busca la fábrica de un modelo.
    pub fn by_model(&self, model: ModelId) -> Option<&dyn DriverFactory> {
        self.factories
            .iter()
            .find(|f| f.model() == model)
            .map(AsRef::as_ref)
    }

    /// Modelos con driver registrado.
    pub fn models(&self) -> Vec<ModelId> {
        self.factories.iter().map(|f| f.model()).collect()
    }

    /// Identidades USB reconocidas (solo las confirmadas).
    pub fn known_device_ids(&self) -> Vec<DeviceId> {
        self.factories
            .iter()
            .filter_map(|f| f.device_id())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    pub fn len(&self) -> usize {
        self.factories.len()
    }
}

impl std::fmt::Debug for DriverRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverRegistry")
            .field("factories", &self.factories.len())
            .field("models", &self.models())
            .finish()
    }
}
