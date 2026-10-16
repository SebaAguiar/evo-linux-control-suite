//! Construcción del registro de drivers y apertura del dispositivo (SAD §1.5).
//!
//! El daemon es el composition root: aquí se decide **qué** adaptador concreto se
//! instala en el [`DriverRegistry`]. Hoy la única implementación disponible es el
//! dispositivo simulado ([`evo_mock`]) porque `evo-usb` está bloqueado hasta la
//! validación con hardware real (SAD §8.7): el protocolo de control del EVO no
//! está documentado y no se inventa. El día que exista, se registra su fábrica
//! junto a la simulada y el núcleo no cambia.

use evo_domain::ModelId;
use evo_mock::MockDriverFactory;
use evo_ports::DriverRegistry;
use evo_ports::EvoDevice;
use evo_ports::ReopenFn;

use crate::DaemonError;

/// Registro con las fábricas disponibles: hardware real EVO 4 vía ALSA si está
/// conectado, o [`MockDriverFactory`] como fallback / para los demás modelos.
pub fn registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();

    if crate::alsa::is_evo4_available("EVO4") {
        tracing::info!(
            "Dispositivo hardware Audient EVO 4 detectado vía ALSA (VID=2708, PID=0006)"
        );
        registry.register(Box::new(crate::alsa::Evo4AlsaDriverFactory::new("EVO4")));
    } else {
        tracing::info!(
            "Audient EVO 4 no detectado físicamente; usando simulador MockDriverFactory"
        );
        registry.register(Box::new(MockDriverFactory::new(ModelId::Evo4)));
    }

    for model in [ModelId::Evo8, ModelId::Evo16] {
        registry.register(Box::new(MockDriverFactory::new(model)));
    }
    registry
}

/// Abre el dispositivo del modelo indicado y devuelve la clausura de reapertura.
///
/// La clausura de *hot-plug* sale de la misma fábrica del registro (SAD §8.6), de
/// modo que `evo-core` puede reintentar la apertura sin conocer el adaptador.
pub fn open(
    registry: &DriverRegistry,
    model: ModelId,
) -> Result<(Box<dyn EvoDevice>, Option<ReopenFn>), DaemonError> {
    let factory = registry
        .by_model(model)
        .ok_or(DaemonError::NoDriver(model))?;
    let reopen = registry.reopen_fn(model);
    let device = factory.open()?;
    Ok((device, reopen))
}
