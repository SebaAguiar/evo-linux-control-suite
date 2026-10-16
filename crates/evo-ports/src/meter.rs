//! Puerto de medidores: canal de alta frecuencia, separado del control.

use evo_domain::{DeviceProfile, MeterError, MeterFrame};

/// Fuente de medidores derivada del flujo de audio (PipeWire/ALSA).
///
/// Es un puerto **independiente de [`crate::EvoDevice`]** a propósito: los
/// medidores se muestrean a 30–60 Hz desde el plano de datos de audio, mientras
/// los comandos viajan por USB a ~1 ms por transferencia. Mezclarlos en un mismo
/// trait obligaría a compartir un lock entre dos ritmos incompatibles
/// (SAD §8.2).
///
/// La implementación concreta (`evo-meter-pipewire`) abre un stream de
/// **monitorización pasivo**, sin fijar *quantum*, para no alterar la política de
/// latencia del servidor de audio (§8.2, riesgo (a)).
pub trait MeterSource: Send {
    /// Arranca la captura para un perfil concreto.
    ///
    /// Idempotente: llamar dos veces no debe abrir dos streams.
    fn start(&mut self, profile: &DeviceProfile) -> Result<(), MeterError>;

    /// Último fotograma disponible. **Nunca bloquea** (SAD §8.2).
    ///
    /// Si todavía no hay datos, debe devolver un fotograma silencioso, no un
    /// error: la ausencia de señal es información válida.
    fn latest(&self) -> MeterFrame;

    /// Detiene la captura y libera el stream.
    fn stop(&mut self) -> Result<(), MeterError> {
        Ok(())
    }
}
