//! Puerto de difusión de eventos hacia los clientes.

use evo_domain::{MeterFrame, StateEvent};

/// Destino de eventos del núcleo (Tauri, sesiones WebSocket, …).
///
/// Se define como trait, y no como un `broadcast` concreto, para que el núcleo no
/// dependa de tokio. El adaptador decide si eso es un `broadcast::Sender`, un
/// `watch` o una simple cola.
///
/// **Nunca debe bloquear**: los emisores reales aplican *throttling* por cliente
/// y descartan a los clientes lentos en lugar de frenar al actor USB
/// (SAD §2.2).
pub trait EventSink: Send + Sync {
    /// Difunde un delta de estado versionado.
    fn emit_state(&self, event: StateEvent);

    /// Difunde un fotograma de medidores.
    ///
    /// Tiene implementación por defecto vacía: los sumideros que no sirven
    /// medidores (por ejemplo un log de auditoría) no necesitan implementarlo.
    fn emit_meters(&self, frame: MeterFrame) {
        let _ = frame;
    }
}
