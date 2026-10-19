//! Puerto de dispositivo: lo que el núcleo necesita de un EVO.

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState,
};

/// Un dispositivo EVO controlable.
///
/// Es un puerto **de salida**: lo implementan los adaptadores (`evo-usb` con
/// `rusb`, `evo-mock` en memoria), nunca el núcleo.
///
/// # Notas de diseño respecto al pseudocódigo del SAD §1.5
///
/// - **No tiene `poll_meters`.** El addendum §8.1 lo retira del trait: los
///   medidores se derivan del audio vía [`crate::MeterSource`], que es un canal
///   separado porque tiene un perfil de frecuencia y de bloqueo distintos.
/// - **No tiene `poll_hw_events` en v1.0.** El equipo decidió que el hardware no
///   notifica cambios físicos y que el `StateStore` es la única verdad
///   (§8.1), así que la lectura de perillas queda detrás del feature
///   `hw-events`.
/// - **`read_state` se mantiene** con implementación por defecto "no
///   soportado", porque la Fase 1 todavía investiga si algún registro permite
///   leer por control transfer (§8.6).
/// - **`apply` devuelve los eventos confirmados.** Es la única forma de que el
///   `StateStore` se entere de lo que realmente pasó, en lugar de asumir que el
///   hardware obedeció.
pub trait EvoDevice: Send {
    /// Perfil estático del modelo: canales, capacidades y rangos.
    fn profile(&self) -> &DeviceProfile;

    /// Aplica un comando y devuelve los eventos confirmados.
    ///
    /// El comando llega **ya validado** por la `SafetyPolicy`; el adaptador se
    /// ocupa de la traducción al protocolo del modelo, no de las reglas de negocio.
    ///
    /// # Errores
    /// - [`DeviceError::Unsupported`] si el modelo no implementa la operación.
    /// - [`DeviceError::Disconnected`] si la unidad se fue durante la llamada.
    fn apply(&mut self, cmd: DeviceCommand) -> Result<Vec<DeviceEvent>, DeviceError>;

    /// Estado de conexión actual.
    fn connection_state(&self) -> ConnectionState {
        ConnectionState::Connected
    }

    /// Relee el estado desde el hardware.
    ///
    /// Por defecto **no está soportado**: es la respuesta honesta mientras la
    /// Fase 1 no confirme que existe un registro legible (§8.6). Un driver que sí
    /// pueda hacerlo lo sobrescribe.
    fn read_state(&mut self) -> Result<DeviceState, DeviceError> {
        Err(DeviceError::Unsupported("read_state"))
    }

    /// Lee cambios físicos (perillas, botones) del hardware.
    ///
    /// Fuera de la ruta v1.0 (SAD §8.1). Solo existe si se compila el feature
    /// `hw-events`.
    #[cfg(feature = "hw-events")]
    fn poll_hw_events(&mut self) -> Result<Vec<DeviceEvent>, DeviceError> {
        Ok(Vec::new())
    }

    /// Cierra el dispositivo liberando el handle.
    fn close(&mut self) -> Result<(), DeviceError> {
        Ok(())
    }

    /// Registra un sumidero de eventos para recibir notificaciones reactivas
    /// directamente desde el hardware (giro físico de perillas, botones, etc.).
    ///
    /// Por defecto no hace nada si el adaptador no soporta push asíncrono.
    fn set_hw_event_sink(&mut self, sink: HwEventCallback) {
        let _ = sink;
    }
}

/// Callback para notificaciones reactivas emitidas directamente por el hardware
/// (p. ej. eventos de mezclador ALSA o interrupt endpoint).
pub type HwEventCallback = Box<dyn Fn(DeviceEvent) + Send + Sync + 'static>;

/// Reapertura de un dispositivo tras una desconexión (SAD §8.6, *hot-plug*).
///
/// Es una clausura y no una referencia a un [`crate::DriverFactory`] porque el
/// actor de `evo-core` vive en su propio hilo y necesita **poseer** la fábrica,
/// no prestarla. `FnMut` y no `Fn` porque un adaptador real puede querer
/// reconectar un `rusb::DeviceHandle` cacheado entre intentos.
///
/// Devuelve `Err` mientras el dispositivo siga ausente: es el caso normal, no
/// una anomalía, y quien llama decide cuánto insistir.
pub type ReopenFn = Box<dyn FnMut() -> Result<Box<dyn EvoDevice>, DeviceError> + Send>;
