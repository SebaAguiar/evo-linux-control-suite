//! Orquestación de EVO Linux Control Suite: bus de comandos, coalescing, actor
//! de dispositivo y almacén de estado.
//!
//! Este crate es el **núcleo** del sistema (SAD §1.2): conoce los
//! [puertos](evo_ports) pero **nunca** un adaptador concreto. El USB real, el
//! dispositivo simulado y el repositorio en disco entran por composición, en los
//! binarios.
//!
//! # Modelo de concurrencia (SAD §2.1)
//!
//! ```text
//!   Tauri / axum  ──submit()──▶  [ canal acotado ]  ──▶  hilo del actor
//!                                                        │  valida (SafetyPolicy)
//!                                                        │  coalesce (ventana 12 ms)
//!                                                        │  aplica (bloqueante)
//!                                                        ▼
//!                                                   StateStore (único escritor)
//!                                                        │  rev monótona
//!                          deltas ◀── broadcast ──────────┤
//!                       snapshot ◀── watch ──────────────┤
//!                    conexión  ◀── watch ────────────────┘
//! ```
//!
//! El actor corre en **su propio hilo** porque `rusb` bloquea: meterlo en el
//! reactor de la aplicación haría que una transferencia lenta congelase la UI.
//!
//! # Por dónde empezar
//!
//! ```no_run
//! # use evo_core::{CoreConfig, EvoCore};
//! # use evo_mock::MockDevice;
//! # use evo_domain::ModelId;
//! # async fn demo() {
//! let core = EvoCore::spawn(Box::new(MockDevice::new(ModelId::Evo4)), CoreConfig::default());
//! let mut snapshots = core.subscribe_snapshots();
//! core.submit(evo_domain::DeviceCommand::SetGain { ch: 0, db: 24.0 })
//!     .await
//!     .expect("comando válido");
//! snapshots.changed().await.expect("delta difundido");
//! # }
//! ```

mod actor;
mod coalescer;
mod safety;
mod sink;
mod store;

use std::sync::Arc;
use std::time::Duration;

use evo_domain::{
    CommandRejection, ConnectionState, DeviceCommand, DeviceState, SafetyPolicy, StateEvent,
    StateSnapshot,
};
use evo_ports::{EventSink, EvoDevice, ReopenFn};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::actor::{Actor, CoreRequest};

pub use coalescer::{Coalescer, DEFAULT_COALESCE_WINDOW};
pub use safety::SafetyRuntime;
pub use sink::{BroadcastSink, STATE_CHANNEL_CAPACITY};
pub use store::StateStore;

/// Política de reconexión tras un *hot-plug* (SAD §8.6).
#[derive(Debug, Clone, Copy)]
pub struct ReconnectPolicy {
    /// Si es `false`, una desconexión es definitiva hasta reiniciar el núcleo.
    pub enabled: bool,
    /// Espera entre intentos de reapertura.
    ///
    /// Se reintenta **indefinidamente** mientras `enabled` sea cierto: enumerar
    /// un bus USB cada `delay` es barato, y rendirse obligaría al usuario a
    /// reiniciar el daemon solo porque desenchufó el cable un rato. El estado de
    /// conexión se queda en `Enumerating`, que es justo lo que la UI quiere decir
    /// ("buscando el dispositivo").
    pub delay: Duration,
    /// ¿Reaplicar ganancia y mute del último estado comandado al reconectar?
    pub replay: bool,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            delay: Duration::from_millis(500),
            replay: true,
        }
    }
}

/// Ajustes del núcleo.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// Ventana de coalescing (SAD §2.2: 8–16 ms).
    pub coalesce_window: Duration,
    /// Capacidad de la cola de comandos. Es **acotada** a propósito: la
    /// contrapresión es preferible a que una ráfaga de cientos de eventos crezca
    /// sin límite.
    pub queue_capacity: usize,
    /// Política de seguridad aplicada a todos los orígenes (SAD §4.3).
    pub policy: SafetyPolicy,
    pub reconnect: ReconnectPolicy,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            coalesce_window: DEFAULT_COALESCE_WINDOW,
            queue_capacity: 256,
            policy: SafetyPolicy::default(),
            reconnect: ReconnectPolicy::default(),
        }
    }
}

/// Error al entregar un comando al núcleo.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CoreError {
    /// La `SafetyPolicy` (o un invariante de dominio) lo rechazó.
    #[error("comando rechazado: {0}")]
    Rejected(#[from] CommandRejection),
    /// El núcleo ya se apagó; no hay actor al que entregar nada.
    #[error("el núcleo se está apagando")]
    Shutdown,
}

/// Núcleo en ejecución.
///
/// Es un **handle clonable por referencia**: los clientes lo comparten y llaman a
/// [`EvoCore::submit`]; las suscripciones se piden una vez al arrancar.
pub struct EvoCore {
    tx: mpsc::Sender<CoreRequest>,
    snapshot: watch::Receiver<StateSnapshot>,
    connection: watch::Receiver<ConnectionState>,
    sink: Arc<BroadcastSink>,
    actor: Option<std::thread::JoinHandle<Box<dyn EvoDevice>>>,
}

impl EvoCore {
    /// Arranca el núcleo sobre un dispositivo, sin capacidad de reconexión.
    pub fn spawn(device: Box<dyn EvoDevice>, config: CoreConfig) -> Self {
        Self::spawn_with_reopen(device, None, config)
    }

    /// Arranca el núcleo con una clausura de reapertura (SAD §8.6).
    ///
    /// El composition root la obtiene de [`evo_ports::DriverRegistry::reopen_fn`]:
    /// así el núcleo reintenta el *hot-plug* sin saber qué adaptador hay detrás.
    pub fn spawn_with_reopen(
        device: Box<dyn EvoDevice>,
        reopen: Option<ReopenFn>,
        config: CoreConfig,
    ) -> Self {
        let profile = device.profile().clone();
        let sink = Arc::new(BroadcastSink::new(profile.inputs));

        let (tx, mut rx) = mpsc::channel(config.queue_capacity.max(1));
        let (snap_tx, snap_rx) = watch::channel(StateSnapshot {
            rev: 0,
            profile: profile.clone(),
            state: DeviceState::initial(&profile),
        });
        let (conn_tx, conn_rx) = watch::channel(ConnectionState::Connected);

        let actor = Actor::new(device, reopen, config, sink.clone(), snap_tx, conn_tx);
        let handle = std::thread::Builder::new()
            .name("evo-device-actor".to_string())
            .spawn(move || {
                // Runtime propio de un solo hilo: da temporizadores al actor sin
                // meterlo en el reactor de la aplicación (SAD §2.1).
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("no se pudo crear el runtime del actor");
                runtime.block_on(actor.run(&mut rx))
            })
            .expect("no se pudo crear el hilo del actor");

        Self {
            tx,
            snapshot: snap_rx,
            connection: conn_rx,
            sink,
            actor: Some(handle),
        }
    }

    /// Envía un comando: se valida, se coalesce y se aplica.
    ///
    /// `Ok(())` significa **aceptado**, no aplicado (SAD §4.6): el resultado real
    /// llega como delta por [`EvoCore::subscribe_state`] o como nueva instantánea
    /// por [`EvoCore::subscribe_snapshots`]. Esa separación es lo que permite
    /// coalescer sin mentir al cliente.
    pub async fn submit(&self, cmd: DeviceCommand) -> Result<(), CoreError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CoreRequest::Submit { cmd, reply })
            .await
            .map_err(|_| CoreError::Shutdown)?;
        rx.await.map_err(|_| CoreError::Shutdown)?
    }

    /// Bloquea o desbloquea el phantom de forma global (SAD §4.3).
    ///
    /// Es la defensa que permite dejar los +48 V fuera de juego mientras se
    /// conectan micrófonos de cinta. Con el bloqueo activo, cualquier
    /// `SetPhantom` se rechaza, venga de donde venga.
    pub async fn set_phantom_lock(&self, locked: bool) -> Result<(), CoreError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CoreRequest::SetPhantomLock { locked, reply })
            .await
            .map_err(|_| CoreError::Shutdown)?;
        rx.await.map_err(|_| CoreError::Shutdown)
    }

    /// Instantánea actual: perfil + estado + `rev`.
    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot.borrow().clone()
    }

    /// Estado de conexión actual.
    pub fn connection(&self) -> ConnectionState {
        *self.connection.borrow()
    }

    /// Suscribe deltas de estado versionados.
    pub fn subscribe_state(&self) -> broadcast::Receiver<StateEvent> {
        self.sink.subscribe_state()
    }

    /// Suscribe instantáneas completas (para clientes que acaban de entrar o se
    /// resincronizan por un salto de `rev`).
    pub fn subscribe_snapshots(&self) -> watch::Receiver<StateSnapshot> {
        self.snapshot.clone()
    }

    /// Suscribe cambios de conexión (efímeros, no forman parte del estado
    /// persistido — SAD §2.4).
    pub fn subscribe_connection(&self) -> watch::Receiver<ConnectionState> {
        self.connection.clone()
    }

    /// Suscribe fotogramas de medidores (los publica el pipeline de audio, no el
    /// actor; el canal lo posee el sumidero).
    pub fn subscribe_meters(&self) -> watch::Receiver<evo_domain::MeterFrame> {
        self.sink.subscribe_meters()
    }

    /// Publica un fotograma de medidores. Lo llama el tarea de medidores, nunca
    /// el camino de comandos (SAD §2.2).
    pub fn publish_meters(&self, frame: evo_domain::MeterFrame) {
        self.sink.emit_meters(frame);
    }

    /// Suscribe eventos como `Arc<dyn EventSink>`, para pasarlo a otros
    /// componentes que publican sin depender de tokio.
    pub fn event_sink(&self) -> Arc<BroadcastSink> {
        Arc::clone(&self.sink)
    }

    /// Apaga el núcleo de forma limpia y devuelve el dispositivo.
    ///
    /// Cerrar el canal hace que el actor aplique lo que quede pendiente y salga;
    /// el hilo devuelve el dispositivo, que sigue siendo utilizable (los tests lo
    /// usan para inspeccionar lo que realmente se aplicó).
    ///
    /// Bloquea hasta que el actor termina: desde contexto async conviene
    /// envolverlo en `spawn_blocking`.
    pub fn shutdown(self) -> Result<Box<dyn EvoDevice>, CoreError> {
        let Self { tx, actor, .. } = self;
        drop(tx);
        match actor {
            Some(handle) => handle.join().map_err(|_| CoreError::Shutdown),
            None => Err(CoreError::Shutdown),
        }
    }
}

impl std::fmt::Debug for EvoCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvoCore")
            .field("rev", &self.snapshot.borrow().rev)
            .field("connection", &self.connection())
            .finish_non_exhaustive()
    }
}
