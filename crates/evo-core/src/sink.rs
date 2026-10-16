//! Sumidero de eventos basado en `tokio::sync`.
//!
//! Implementa el puerto [`EventSink`] **sin** que el puerto dependa de tokio
//! (SAD §1.2): el núcleo conoce el trait, y la composición elige este sumidero
//! concreto.
//!
//! El reparto sigue el SAD §2.1:
//!
//! - **Estado** → `broadcast`. Varios consumidores, cada uno con su ritmo; un
//!   cliente lento no puede frenar al actor, y si se atrasa demasiado recibe
//!   `Lagged` y se resincroniza con [`crate::EvoCore::snapshot`].
//! - **Medidores** → `watch`. Solo importa el último fotograma; acumular 60 Hz de
//!   historia no tiene sentido y `watch` descarta lo viejo gratis.

use evo_domain::{MeterFrame, StateEvent};
use evo_ports::EventSink;
use tokio::sync::{broadcast, watch};

/// Capacidad del canal de deltas de estado.
///
/// Acotado a propósito (SAD §2.2: colas acotadas). Si un cliente se queda atrás
/// más de esto, `Lagged` es la señal correcta: debe pedir un snapshot en lugar de
/// intentar ponerse al día con deltas parciales.
pub const STATE_CHANNEL_CAPACITY: usize = 256;

/// Sumidero que reparte deltas de estado y fotogramas de medidores.
#[derive(Debug)]
pub struct BroadcastSink {
    state: broadcast::Sender<StateEvent>,
    meters: watch::Sender<MeterFrame>,
}

impl BroadcastSink {
    /// Crea el sumidero, con un fotograma de medidores inicial silencioso.
    pub fn new(channels: u8) -> Self {
        let (state, _) = broadcast::channel(STATE_CHANNEL_CAPACITY);
        let (meters, _) = watch::channel(MeterFrame::silent(0, channels));
        Self { state, meters }
    }

    /// Suscribe un consumidor a los deltas de estado.
    pub fn subscribe_state(&self) -> broadcast::Receiver<StateEvent> {
        self.state.subscribe()
    }

    /// Suscribe un consumidor a los fotogramas de medidores.
    pub fn subscribe_meters(&self) -> watch::Receiver<MeterFrame> {
        self.meters.subscribe()
    }

    /// ¿Hay alguien escuchando los deltas?
    ///
    /// Permite al actor saltarse el trabajo de construir el evento cuando no hay
    /// clientes, que es el caso común en el daemon de escritorio.
    pub fn has_state_subscribers(&self) -> bool {
        self.state.receiver_count() > 0
    }

    pub fn has_meter_subscribers(&self) -> bool {
        self.meters.receiver_count() > 0
    }
}

impl EventSink for BroadcastSink {
    fn emit_state(&self, event: StateEvent) {
        // `send` falla si no hay receptores: es normal, no un error.
        let _ = self.state.send(event);
    }

    fn emit_meters(&self, frame: MeterFrame) {
        let _ = self.meters.send(frame);
    }
}
