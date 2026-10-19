//! Actor de dispositivo: el único que habla con el hardware.
//!
//! Materializa el modelo de concurrencia del SAD §2.1: **un hilo dedicado** posee
//! el handle USB (rusb bloquea, así que no puede vivir en el reactor de la
//! aplicación) y todo lo demás le habla por mensajes. El actor es también el
//! **único escritor** del [`StateStore`], con lo que el invariante de "una sola
//! verdad" no depende de candados compartidos.
//!
//! El bucle tiene tres responsabilidades, en este orden:
//!
//! 1. **Validar** cada comando contra la `SafetyPolicy` (nada llega al hardware sin
//!    pasar por ahí).
//! 2. **Coalescer** las ráfagas antes de tocar el bus ([`Coalescer`]).
//! 3. **Publicar** los deltas confirmados y reconectar si el dispositivo se va.
//!
//! El tiempo se lee de un origen monótono propio ([`Instant`]) y se traduce a
//! `Duration`, que es lo que la política pura espera.
//!
//! Los comandos pendientes **no tienen una cola propia**: si el dispositivo está
//! caído no se aplican y siguen en el coalescer hasta que reconecte. Eso evita
//! duplicar el orden y garantiza que nada se pierde ni se aplica dos veces.

use std::sync::Arc;
use std::time::{Duration, Instant};

use evo_domain::{
    ConnectionState, DeviceCommand, DeviceError, DeviceEvent, SmartGainPhase, StateOp,
    StateSnapshot, state_ops_from_events,
};
use evo_ports::{EventSink, EvoDevice, ReopenFn};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, warn};

use crate::coalescer::Coalescer;
use crate::safety::SafetyRuntime;
use crate::store::StateStore;
use crate::{CoreConfig, CoreError};

/// Petición dirigida al actor.
pub(crate) enum CoreRequest {
    /// Valida y encola un comando. La confirmación es "aceptado", no "aplicado":
    /// el resultado real viaja como delta de estado (SAD §4.6).
    Submit {
        cmd: DeviceCommand,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    /// Conmuta el bloqueo global de phantom (SAD §4.3).
    ///
    /// Es una petición y no un comando de dispositivo porque **no** viaja al
    /// hardware: cambia una regla del núcleo. Tiene que pasar por el actor
    /// igualmente, porque es el único que posee el estado de seguridad.
    SetPhantomLock {
        locked: bool,
        reply: oneshot::Sender<()>,
    },
    /// Fuerza una reconciliación inmediata del estado contra el hardware físico ([`EvoDevice::read_state`]).
    Resync { reply: oneshot::Sender<()> },
}

/// Origen del despertar del bucle del actor.
enum WakeSource {
    Request(Option<CoreRequest>),
    HwEvent(DeviceEvent),
}

/// Espera concurrente por petición de software o evento reactivo de hardware.
async fn next_wake(
    rx: &mut mpsc::Receiver<CoreRequest>,
    hw_rx: &mut mpsc::UnboundedReceiver<DeviceEvent>,
) -> WakeSource {
    tokio::select! {
        biased;
        req = rx.recv() => WakeSource::Request(req),
        Some(hw_ev) = hw_rx.recv() => WakeSource::HwEvent(hw_ev),
    }
}

/// Resultado de intentar aplicar un comando al hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyOutcome {
    Applied,
    /// La transferencia falló pero el dispositivo sigue vivo.
    Failed,
    /// El dispositivo se fue: hay que reconectar.
    Disconnected,
}

/// Estado y bucle del actor.
pub(crate) struct Actor {
    device: Box<dyn EvoDevice>,
    reopen: Option<ReopenFn>,
    store: StateStore,
    safety: SafetyRuntime,
    coalescer: Coalescer,
    sink: Arc<dyn EventSink>,
    snapshots: watch::Sender<StateSnapshot>,
    connection: watch::Sender<ConnectionState>,
    config: CoreConfig,
    origin: Instant,
    poll_quiet_until: Duration,
    hw_tx: mpsc::UnboundedSender<DeviceEvent>,
    hw_rx: mpsc::UnboundedReceiver<DeviceEvent>,
}

impl Actor {
    pub(crate) fn new(
        mut device: Box<dyn EvoDevice>,
        reopen: Option<ReopenFn>,
        config: CoreConfig,
        sink: Arc<dyn EventSink>,
        snapshots: watch::Sender<StateSnapshot>,
        connection: watch::Sender<ConnectionState>,
    ) -> Self {
        let profile = device.profile().clone();
        let store = StateStore::initial(profile);
        let safety = SafetyRuntime::new(config.policy);
        let (hw_tx, hw_rx) = mpsc::unbounded_channel();
        let hw_tx_cb = hw_tx.clone();
        device.set_hw_event_sink(Box::new(move |event| {
            let _ = hw_tx_cb.send(event);
        }));

        Self {
            device,
            reopen,
            store,
            safety,
            coalescer: Coalescer::new(config.coalesce_window),
            sink,
            snapshots,
            connection,
            config,
            origin: Instant::now(),
            poll_quiet_until: Duration::ZERO,
            hw_tx,
            hw_rx,
        }
    }

    /// Reloj monótono del actor, como `Duration` desde su arranque.
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    /// Bucle principal. Devuelve el dispositivo al terminar, para que el
    /// propietario pueda inspeccionarlo o reutilizarlo tras un apagado limpio.
    pub(crate) async fn run(mut self, rx: &mut mpsc::Receiver<CoreRequest>) -> Box<dyn EvoDevice> {
        if self.connection_value() == ConnectionState::Connected {
            self.poll_hardware();
        }

        loop {
            // 1) Drena todo lo que haya en cola sin bloquear. Durante una ráfaga
            //    (un slider arrastrándose) el actor está "fuera" aplicando el
            //    comando anterior, así que aquí se acumula el trabajo pendiente.
            loop {
                match rx.try_recv() {
                    Ok(req) => self.handle(req),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        // Apagado limpio: se aplica lo que quede pendiente antes
                        // de soltar el dispositivo.
                        self.flush().await;
                        return self.device;
                    }
                }
            }
            loop {
                match self.hw_rx.try_recv() {
                    Ok(hw_event) => self.handle_hw_event(hw_event),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            let connected = self.connection_value() == ConnectionState::Connected;

            // 2) Con el dispositivo vivo, si la ventana de coalescing venció se
            //    aplica lo acumulado.
            if connected && self.coalescer.has_due(self.now()) {
                if !self.flush().await {
                    self.begin_reconnect().await;
                }
                continue;
            }

            // 3) Espera. Con dispositivo, hasta que venza la ventana (o llegue
            //    trabajo). Sin dispositivo, hasta el próximo intento de
            //    reconexión: **siempre con temporizador**, para que este bucle no
            //    se convierta en un giro al 100 % de CPU.
            let poll_interval = Duration::from_millis(1500);
            let until_due = if connected {
                self.coalescer.time_until_due(self.now())
            } else {
                None
            };

            match until_due {
                Some(remaining) if !self.coalescer.is_empty() => {
                    let wait = remaining.min(poll_interval);
                    match tokio::time::timeout(wait, next_wake(rx, &mut self.hw_rx)).await {
                        Ok(WakeSource::Request(Some(req))) => self.handle(req),
                        Ok(WakeSource::Request(None)) => {
                            self.flush().await;
                            return self.device;
                        }
                        Ok(WakeSource::HwEvent(hw_event)) => self.handle_hw_event(hw_event),
                        Err(_) => {
                            if self.coalescer.has_due(self.now()) {
                                if !self.flush().await {
                                    self.begin_reconnect().await;
                                }
                            } else if connected {
                                self.poll_hardware();
                            }
                        }
                    }
                }
                _ if !connected && self.config.reconnect.enabled => {
                    match tokio::time::timeout(self.config.reconnect.delay, rx.recv()).await {
                        Ok(Some(req)) => self.handle(req),
                        Ok(None) => return self.device,
                        Err(_) => self.try_reconnect().await,
                    }
                }
                _ => {
                    let wait = if connected {
                        let now = self.now();
                        if now < self.poll_quiet_until {
                            (self.poll_quiet_until - now).max(Duration::from_millis(100))
                        } else {
                            poll_interval
                        }
                    } else {
                        Duration::from_secs(3600)
                    };
                    match tokio::time::timeout(wait, next_wake(rx, &mut self.hw_rx)).await {
                        Ok(WakeSource::Request(Some(req))) => self.handle(req),
                        Ok(WakeSource::Request(None)) => {
                            if connected {
                                self.flush().await;
                            }
                            return self.device;
                        }
                        Ok(WakeSource::HwEvent(hw_event)) => self.handle_hw_event(hw_event),
                        Err(_) => {
                            if connected {
                                self.poll_hardware();
                            }
                        }
                    }
                }
            }
        }
    }

    /// Atiende una petición. Nunca bloquea: encolar y responder es todo.
    fn handle(&mut self, req: CoreRequest) {
        match req {
            CoreRequest::Submit { cmd, reply } => self.submit(cmd, reply),
            CoreRequest::SetPhantomLock { locked, reply } => {
                self.safety.set_phantom_locked(locked);
                let _ = reply.send(());
            }
            CoreRequest::Resync { reply } => {
                self.poll_hardware();
                let _ = reply.send(());
            }
        }
    }

    /// Procesa un evento reactivo originado en el hardware físico (p. ej. ALSA numid=3 o perilla rotativa).
    fn handle_hw_event(&mut self, hw_event: DeviceEvent) {
        if self.connection_value() != ConnectionState::Connected {
            return;
        }

        if let Some(op) = hw_event.to_state_op() {
            match self.store.apply(&[op]) {
                Ok(Some(event)) => {
                    for op in &event.ops {
                        match op {
                            StateOp::Phantom { ch, on } => {
                                self.safety.note_applied(
                                    &DeviceCommand::SetPhantom { ch: *ch, on: *on },
                                    self.now(),
                                );
                            }
                            StateOp::SmartGain { phase } => {
                                self.safety.set_smartgain_running(*phase != SmartGainPhase::Idle);
                            }
                            _ => {}
                        }
                    }
                    self.sink.emit_state(event);
                    let _ = self.snapshots.send(self.store.snapshot());
                }
                Ok(None) => {
                    // El valor notificado coincide con el estado actual (eco o idempotente).
                    // No se avanza la revisión ni se emite ruido.
                }
                Err(err) => {
                    debug!(%err, "error al aplicar evento reactivo de hardware en StateStore");
                }
            }
        } else if let DeviceEvent::ConnectionChanged(conn) = hw_event {
            self.set_connection(conn);
        }
    }

    /// Consulta el estado físico del dispositivo si este lo soporta ([`EvoDevice::read_state`]).
    /// Si difiere del estado comandado (p. ej. botones presionados físicamente en la placa),
    /// actualiza el store y emite los deltas correspondientes.
    fn poll_hardware(&mut self) {
        if self.connection_value() != ConnectionState::Connected {
            return;
        }

        // Si se acaba de enviar un comando al hardware (por ejemplo conmutación de relé 48V
        // o rampa del convertidor elevador DC-DC), dejamos un periodo de reposo en el endpoint 0
        // para evitar saturar el microcontrolador y provocar un reinicio por watchdog de la EVO 4.
        if self.now() < self.poll_quiet_until {
            return;
        }

        match self.device.read_state() {
            Ok(hw_state) => {
                if let Some(event) = self.store.replace(hw_state) {
                    for op in &event.ops {
                        if let StateOp::Phantom { ch, on } = op {
                            self.safety.note_applied(
                                &DeviceCommand::SetPhantom { ch: *ch, on: *on },
                                self.now(),
                            );
                        }
                    }
                    self.sink.emit_state(event);
                    let _ = self.snapshots.send(self.store.snapshot());
                }
            }
            Err(DeviceError::Unsupported(_)) => {}
            Err(DeviceError::Disconnected) => {
                self.set_connection(ConnectionState::Disconnected);
            }
            Err(err) => {
                debug!(%err, "fallo al consultar estado físico del dispositivo");
            }
        }
    }

    /// Valida un comando y lo encola para aplicar. Responde de inmediato.
    fn submit(&mut self, cmd: DeviceCommand, reply: oneshot::Sender<Result<(), CoreError>>) {
        let now = self.now();

        let result = self
            .safety
            .validate(cmd, self.store.profile(), self.store.state(), now)
            .map(|cmd| self.coalescer.push(cmd, now))
            .map_err(CoreError::Rejected);

        let _ = reply.send(result);
    }

    /// Aplica lo acumulado, en orden.
    ///
    /// Devuelve `false` si el dispositivo se fue a mitad del lote: en ese caso
    /// quien llama decide reconectar (el bucle principal), en lugar de anidar la
    /// reconexión aquí y volver a llamar a `flush` —lo que sería una recursión
    /// asíncrona infinita—.
    async fn flush(&mut self) -> bool {
        if self.connection_value() != ConnectionState::Connected {
            // Nada sale hacia un dispositivo caído: lo pendiente espera en el
            // coalescer a que reconecte.
            return true;
        }

        let batch = self.coalescer.flush();
        for (idx, cmd) in batch.iter().enumerate() {
            let quiet_duration = match cmd {
                DeviceCommand::SetPhantom { .. } => Duration::from_millis(2500),
                _ => Duration::from_millis(600),
            };
            let now = self.now();
            self.poll_quiet_until = self.poll_quiet_until.max(now + quiet_duration);

            match self.apply_one(*cmd) {
                ApplyOutcome::Applied | ApplyOutcome::Failed => {}
                ApplyOutcome::Disconnected => {
                    // Reencola este y los siguientes: siguen siendo la última
                    // intención del usuario y se aplicarán tras reconectar.
                    let now = self.now();
                    for rest in &batch[idx..] {
                        self.coalescer.push(*rest, now);
                    }
                    self.set_connection(ConnectionState::Disconnected);
                    return false;
                }
            }
        }
        true
    }

    /// Aplica un comando y publica sus confirmaciones.
    fn apply_one(&mut self, cmd: DeviceCommand) -> ApplyOutcome {
        match self.device.apply(cmd) {
            Ok(events) => {
                let now = self.now();
                self.safety.note_applied(&cmd, now);
                self.publish_events(&events);
                ApplyOutcome::Applied
            }
            Err(DeviceError::Disconnected) => ApplyOutcome::Disconnected,
            Err(err) => {
                // Un fallo de transferencia no mata el dispositivo: se registra y
                // se sigue. El estado del store no cambia porque el hardware no
                // obedeció, que es exactamente lo que se quiere.
                warn!(comando = cmd.op_name(), %err, "transferencia fallida");
                ApplyOutcome::Failed
            }
        }
    }

    /// Traduce confirmaciones del adaptador en deltas de estado y avisos.
    fn publish_events(&mut self, events: &[DeviceEvent]) {
        for ev in events {
            match ev {
                DeviceEvent::SmartGainChanged { phase } => {
                    self.safety.note_smartgain_phase(*phase);
                }
                DeviceEvent::ConnectionChanged(state) => self.set_connection(*state),
                _ => {}
            }
        }

        let ops: Vec<StateOp> = state_ops_from_events(events);
        self.commit(&ops);
    }

    /// Aplica deltas al store y difunde el cambio, si lo hay.
    fn commit(&mut self, ops: &[StateOp]) {
        match self.store.apply(ops) {
            Ok(Some(event)) => {
                self.sink.emit_state(event);
                let _ = self.snapshots.send(self.store.snapshot());
            }
            Ok(None) => {}
            Err(err) => {
                // Solo puede pasar si un adaptador devuelve un índice fuera del
                // perfil, y entonces el contrato está roto.
                warn!(%err, "delta de estado inválido descartado");
            }
        }
    }

    fn connection_value(&self) -> ConnectionState {
        *self.connection.borrow()
    }

    fn set_connection(&mut self, state: ConnectionState) {
        if self.connection_value() != state {
            let _ = self.connection.send(state);
        }
    }

    /// Se perdió el dispositivo: se conserva el estado comandado y se pasa a
    /// `Enumerating`, desde donde el bucle principal reintentará (SAD §8.5, §8.6).
    async fn begin_reconnect(&mut self) {
        if !self.config.reconnect.enabled || self.reopen.is_none() {
            // Sin política de reconexión, la desconexión es definitiva.
            self.set_connection(ConnectionState::Disconnected);
            return;
        }
        debug!("dispositivo desconectado; se reintentará reabrir");
        self.set_connection(ConnectionState::Enumerating);
        self.try_reconnect().await;
    }

    /// Un único intento de reapertura, con reaplicación del estado si sale bien.
    ///
    /// Un intento por llamada, no una ráfaga: los reintentos los espacia el bucle
    /// principal, y así el actor nunca se queda durante segundos sin atender lo
    /// que llega por la cola.
    async fn try_reconnect(&mut self) {
        let opened = match self.reopen.as_mut() {
            Some(reopen) => reopen(),
            None => return,
        };

        match opened {
            Ok(mut device) => {
                debug!("dispositivo reabierto");
                let hw_tx_cb = self.hw_tx.clone();
                device.set_hw_event_sink(Box::new(move |event| {
                    let _ = hw_tx_cb.send(event);
                }));
                self.device = device;
                self.replay_state();
                self.set_connection(ConnectionState::Connected);
                self.flush().await;
                self.poll_hardware();
            }
            Err(err) => {
                debug!(%err, "reapertura fallida; se reintentará");
                // Se permanece en `Enumerating`: la UI muestra "buscando", que es
                // exactamente lo que está pasando.
            }
        }
    }

    /// Reaplica ganancia y mute del último estado comandado.
    ///
    /// El **phantom no se reaplica** (SAD §8.5): reencender 48 V sin que nadie lo
    /// pida es justo el daño que la política de seguridad existe para evitar. Un
    /// comando explícito llegado mientras el equipo estaba caído sí se aplica,
    /// porque es una intención del usuario, no una restauración automática: y ese
    /// comando espera en el coalescer, no aquí.
    fn replay_state(&mut self) {
        if !self.config.reconnect.replay {
            return;
        }
        let state = self.store.state().clone();
        for (i, channel) in state.channels.iter().enumerate() {
            let ch = i as u8;
            let _ = self.device.apply(DeviceCommand::SetGain {
                ch,
                db: channel.gain_db,
            });
            let _ = self.device.apply(DeviceCommand::SetMute {
                ch,
                on: channel.muted,
            });
        }
    }
}
