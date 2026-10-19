//! EVO Linux Control Suite — daemon sin interfaz gráfica.
//!
//! Es la **raíz de composición** del sistema (SAD §8.3): el único proceso que
//! posee el dispositivo, encima del cual se montan la [persistencia](evo_persist)
//! y el [servidor remoto](evo_remote). La UI de Tauri será *otro cliente* del
//! mismo contrato, no el dueño del hardware.
//!
//! ```text
//!   [server].enabled ──▶ serve(LAN IP)      ┐
//!                      ──▶ serve(loopback)  ├─▶ Arc<EvoCore> ──▶ hilo del actor
//!   GET/PUT config   ◀── FileStateRepo      ┘
//! ```
//!
//! # Qué hace este crate y qué no
//!
//! - **Hace:** componer registro → dispositivo → núcleo → persistencia → red;
//!   reaplicar el último estado al arrancar; guardar con *debounce*; tomar el
//!   bloqueo de instancia única; resolver la interfaz de LAN.
//! - **No hace:** hablar USB real (`evo-usb` está bloqueado, SAD §8.7) ni editar
//!   `config.toml` desde fuera (el daemon es el único escritor, SAD §8.5).

pub mod alsa;
pub mod cli;
mod device;
pub mod lock;
pub mod meter;
pub mod net;
mod persist;

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use evo_core::CoreConfig;
use evo_core::EvoCore;
use evo_domain::DeviceCommand;
use evo_domain::DeviceError;
use evo_domain::ModelId;
use evo_domain::PersistedState;
use evo_domain::ServerConfig;
use evo_persist::FileStateRepo;
use evo_ports::StateRepo;
use evo_remote::RemoteConfig;
use evo_remote::RemoteHandle;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use lock::LockError;
pub use net::NetError;

/// Ajustes de arranque del daemon.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Modelo a controlar.
    pub model: ModelId,
    /// Ruta del documento `config.toml` (ajustes + estado).
    pub state_path: PathBuf,
    /// Ruta del fichero de bloqueo de instancia única.
    pub lock_path: PathBuf,
    /// ¿Tomar el bloqueo? Los tests de composición lo desactivan para poder
    /// montar varios daemons en paralelo sobre directorios temporales.
    pub acquire_lock: bool,
    /// Ajustes del servidor remoto que **pisan** los de `config.toml`.
    ///
    /// Los usa la línea de órdenes (`--remote-interface`, `--remote-port`) sin
    /// obligar a editar el archivo a mano.
    pub server_override: Option<ServerConfig>,
    /// Reenvía a [`RemoteConfig::allow_public_peers`] (SAD §8.4).
    pub allow_public_peers: bool,
    /// Ventana de *debounce* del guardado de estado (SAD §8.5).
    pub debounce: Duration,
    /// Ajustes del núcleo (coalescing, seguridad, reconexión).
    pub core: CoreConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            model: ModelId::Evo4,
            state_path: evo_persist::xdg::config_path(),
            lock_path: default_lock_path(),
            acquire_lock: true,
            server_override: None,
            allow_public_peers: false,
            debounce: Duration::from_secs(2),
            core: CoreConfig::default(),
        }
    }
}

/// Ruta por defecto del bloqueo: `$XDG_RUNTIME_DIR/evo-daemon.lock` (SAD §8.3).
///
/// El runtime dir es efímero y se vacía al cerrar la sesión, que es justo lo que
/// se quiere para un bloqueo. Si no está definido, se cae a `/tmp`.
pub fn default_lock_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    dir.join("evo-daemon.lock")
}

/// Error de arranque o parada del daemon.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// No hay driver registrado para el modelo pedido.
    #[error("no hay driver registrado para el modelo `{}`", .0.code())]
    NoDriver(ModelId),
    /// El adaptador no pudo abrir el dispositivo.
    #[error("no se pudo abrir el dispositivo: {0}")]
    Open(#[from] DeviceError),
    /// No se pudo escuchar en la dirección pedida.
    #[error("no se pudo enlazar el servidor remoto en {addr}: {source}")]
    Serve {
        /// Dirección que se intentó enlazar.
        addr: SocketAddr,
        /// Causa del sistema.
        #[source]
        source: std::io::Error,
    },
    /// La interfaz de LAN no se pudo resolver.
    #[error("interfaz de LAN `{interface}`: {source}")]
    Interface {
        /// Nombre de la interfaz pedida.
        interface: String,
        /// Causa.
        #[source]
        source: NetError,
    },
    /// No se pudo tomar el bloqueo de instancia única.
    #[error(transparent)]
    Instance(#[from] LockError),
}

/// Daemon en ejecución.
pub struct Daemon {
    core: Arc<EvoCore>,
    loopback: RemoteHandle,
    lan: Option<RemoteHandle>,
    persist_stop: Option<oneshot::Sender<()>>,
    persist_task: JoinHandle<()>,
    meter_stop: Option<oneshot::Sender<()>>,
    meter_task: Option<JoinHandle<()>>,
    _lock: Option<lock::SingleInstance>,
}

impl Daemon {
    /// Compone y arranca el daemon.
    ///
    /// Orden deliberado: primero el bloqueo (para no tocar el USB desde dos
    /// procesos), luego el documento (para reaplicar el estado), después el
    /// núcleo, y por último la red; así ningún cliente puede hablar con un núcleo
    /// que aún no está listo.
    pub async fn start(config: DaemonConfig) -> Result<Self, DaemonError> {
        let _lock = if config.acquire_lock {
            Some(lock::SingleInstance::acquire(&config.lock_path)?)
        } else {
            None
        };

        // 1. Documento persistido: ajustes `[server]` + último estado comandado.
        let repo = FileStateRepo::new(&config.state_path);
        let loaded = match repo.load() {
            Ok(doc) => doc,
            // No poder leer el documento no debe impedir controlar el hardware
            // (SAD §8.5): se arranca sin estado y se registra el problema.
            Err(err) => {
                tracing::warn!(error = %err, "config.toml ilegible; se arranca sin estado");
                None
            }
        };

        // 2. Dispositivo y núcleo.
        let registry = device::registry();
        let (device, reopen) = device::open(&registry, config.model)?;
        let is_hw = alsa::is_evo4_available("EVO4") && config.model == ModelId::Evo4;
        tracing::info!(
            model = config.model.code(),
            driver = if is_hw {
                "hardware ALSA (EVO 4 real)"
            } else {
                "simulador evo-mock"
            },
            "dispositivo inicializado en el núcleo"
        );
        let core = Arc::new(EvoCore::spawn_with_reopen(device, reopen, config.core));

        // 3. Reaplicar el estado persistido (ganancia y mute; el phantom no).
        if let Some(doc) = &loaded {
            persist::replay(&core, doc).await;
        }

        // 4. Red: ajustes de `[server]`, con posible pisotón de la CLI.
        let server = config
            .server_override
            .clone()
            .or_else(|| loaded.as_ref().map(|doc| doc.server.clone()))
            .unwrap_or_default();
        let (loopback, lan) = serve(&core, &server, config.allow_public_peers).await?;

        // 5. Guardado con *debounce*.
        let (persist_stop, persist_rx) = oneshot::channel();
        let persist_task = tokio::spawn(persist::run(
            Arc::clone(&core),
            repo,
            server,
            config.debounce,
            persist_rx,
        ));

        // 6. Tarea de medidores de audio en tiempo real (~30 FPS).
        let profile = registry
            .by_model(config.model)
            .map(|f| f.profile())
            .unwrap_or_else(evo_domain::DeviceProfile::evo4);
        let mut meter_source = meter::create_meter_source(is_hw, &profile);
        let (meter_stop_tx, mut meter_stop_rx) = oneshot::channel();
        let meter_core = Arc::clone(&core);
        let meter_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(33));
            loop {
                tokio::select! {
                    _ = &mut meter_stop_rx => break,
                    _ = interval.tick() => {
                        let frame = meter_source.latest();
                        meter_core.publish_meters(frame);
                    }
                }
            }
            let _ = meter_source.stop();
        });

        Ok(Self {
            core,
            loopback,
            lan,
            persist_stop: Some(persist_stop),
            persist_task,
            meter_stop: Some(meter_stop_tx),
            meter_task: Some(meter_task),
            _lock,
        })
    }

    /// Núcleo en ejecución, para tests y para montar más clientes.
    pub fn core(&self) -> &Arc<EvoCore> {
        &self.core
    }

    /// Dirección real del listener de loopback (útil con el puerto 0).
    pub fn loopback_addr(&self) -> SocketAddr {
        self.loopback.local_addr()
    }

    /// Dirección real del listener de LAN, si se enlazó.
    pub fn lan_addr(&self) -> Option<SocketAddr> {
        self.lan.as_ref().map(RemoteHandle::local_addr)
    }

    /// Apaga de forma ordenada: red, volcado final, núcleo y bloqueo.
    pub async fn shutdown(self) {
        let Self {
            core,
            loopback,
            lan,
            persist_stop,
            persist_task,
            meter_stop,
            meter_task,
            _lock,
        } = self;

        // 1. Cortar la red primero: no deben entrar más comandos.
        if let Some(lan) = lan {
            lan.shutdown().await;
        }
        loopback.shutdown().await;

        // 2. Detener la tarea de medidores.
        if let Some(stop) = meter_stop {
            let _ = stop.send(());
        }
        if let Some(task) = meter_task {
            if let Err(err) = task.await {
                tracing::warn!(error = %err, "la tarea de medidores terminó de forma anómala");
            }
        }

        // 3. Pedir el volcado final y esperar a que la tarea termine.
        if let Some(stop) = persist_stop {
            let _ = stop.send(());
        }
        if let Err(err) = persist_task.await {
            tracing::warn!(error = %err, "la tarea de persistencia terminó de forma anómala");
        }

        // 4. Apagar el núcleo y unir el hilo del actor. `shutdown` es síncrono y
        //    bloquea hasta que el actor sale, así que fuera del reactor.
        match Arc::try_unwrap(core) {
            Ok(core) => {
                if tokio::task::spawn_blocking(move || core.shutdown())
                    .await
                    .is_err()
                {
                    tracing::warn!("no se pudo unir el hilo del actor al apagar");
                }
            }
            Err(_) => {
                // Alguna sesión de WebSocket conserva una referencia; al salir el
                // proceso, el canal se cierra y el actor termina solo.
                tracing::warn!("quedan referencias al núcleo; el actor se cerrará al salir");
            }
        }
        // `_lock` se libera aquí, al final de todo.
    }
}

/// Enlaza loopback y, si procede, la IP de LAN (SAD §8.3, §8.4).
async fn serve(
    core: &Arc<EvoCore>,
    server: &ServerConfig,
    allow_public_peers: bool,
) -> Result<(RemoteHandle, Option<RemoteHandle>), DaemonError> {
    let loopback_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), server.port);
    let lan_addr = if server.enabled {
        match net::lan_ipv4_or_default(server.interface.as_deref()) {
            Ok((iface_name, ip)) => {
                let addr = SocketAddr::new(IpAddr::V4(ip), server.port);
                tracing::info!(
                    interface = %iface_name,
                    ip = %ip,
                    port = server.port,
                    "interfaz y dirección de red resueltas para el servidor remoto"
                );
                Some(addr)
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "[server].enabled = true pero no se pudo resolver una IP de LAN utilizable; solo se escucha en loopback"
                );
                None
            }
        }
    } else {
        None
    };

    let loopback = evo_remote::serve(
        Arc::clone(core),
        RemoteConfig {
            bind: loopback_addr,
            lan_addr,
            allow_public_peers,
            ..RemoteConfig::default()
        },
    )
    .await
    .map_err(|source| DaemonError::Serve {
        addr: loopback_addr,
        source,
    })?;

    let lan = if let Some(addr) = lan_addr {
        let handle = evo_remote::serve(
            Arc::clone(core),
            RemoteConfig {
                bind: addr,
                lan_addr: Some(addr),
                allow_public_peers,
                ..RemoteConfig::default()
            },
        )
        .await
        .map_err(|source| DaemonError::Serve { addr, source })?;
        tracing::info!(%addr, "servidor remoto enlazado en la LAN");
        Some(handle)
    } else {
        None
    };

    Ok((loopback, lan))
}

/// Reaplica el último estado comandado, ignorando el phantom (SAD §8.5).
///
/// La ganancia y el mute son seguros de restaurar; el phantom requiere
/// confirmación explícita y nunca se reaplica solo.
pub(crate) fn replay_commands(doc: &PersistedState, inputs: u8) -> Vec<DeviceCommand> {
    let mut commands = Vec::new();
    for (ch, channel) in doc.state.channels.iter().enumerate().take(inputs as usize) {
        let ch = ch as u8;
        commands.push(DeviceCommand::SetGain {
            ch,
            db: channel.gain_db,
        });
        commands.push(DeviceCommand::SetMute {
            ch,
            on: channel.muted,
        });
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use evo_domain::DeviceProfile;
    use evo_domain::DeviceState;

    #[test]
    fn el_replay_cubre_ganancia_y_mute_pero_no_el_phantom() {
        let profile = DeviceProfile::evo8();
        let mut state = DeviceState::initial(&profile);
        state.channels[0].gain_db = 24.0;
        state.channels[0].muted = true;
        state.channels[0].phantom = true; // no debe reaplicarse

        let doc = PersistedState::new(profile, state, 0);
        let commands = replay_commands(&doc, 2);

        // Dos canales dentro del límite `inputs = 2`: 2 comandos cada uno.
        assert_eq!(commands.len(), 4);
        assert!(
            commands
                .iter()
                .all(|cmd| !matches!(cmd, DeviceCommand::SetPhantom { .. }))
        );
        assert!(commands.contains(&DeviceCommand::SetGain { ch: 0, db: 24.0 }));
        assert!(commands.contains(&DeviceCommand::SetMute { ch: 0, on: true }));
    }

    #[test]
    fn el_replay_respeta_el_numero_de_canales_del_dispositivo() {
        let persisted = DeviceProfile::evo16();
        let state = DeviceState::initial(&persisted);
        let doc = PersistedState::new(persisted, state, 0);

        // El dispositivo abierto es un EVO 4 (2 entradas): solo se tocan 2.
        let commands = replay_commands(&doc, 2);
        assert_eq!(commands.len(), 4);
        assert!(commands.iter().all(|cmd| cmd.channel().unwrap_or(0) < 2));
    }

    #[tokio::test]
    async fn el_daemon_publica_fotogramas_de_medidores_por_websocket() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("config.toml");
        let lock_path = tmp.path().join("lock");

        let mut config = DaemonConfig::default();
        config.state_path = state_path;
        config.lock_path = lock_path;
        config.acquire_lock = false;
        config.server_override = Some(ServerConfig {
            enabled: false,
            interface: None,
            port: 0,
        });

        let daemon = Daemon::start(config).await.expect("daemon arranca");
        let ws_url = format!("ws://{}/ws", daemon.loopback_addr());

        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.expect("ws conecta");
        use futures_util::StreamExt;

        let mut received_binary_meter = false;
        for _ in 0..10 {
            let msg = tokio::time::timeout(Duration::from_millis(500), ws.next())
                .await
                .expect("timeout esperando frame")
                .expect("stream cerrado")
                .expect("error en ws");
            if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg {
                let frame = evo_domain::MeterFrame::decode(&bytes).expect("frame decodifica");
                assert!(frame.channels() >= 2);
                received_binary_meter = true;
                break;
            }
        }

        assert!(received_binary_meter, "debe recibirse fotograma binario de medidores");
        daemon.shutdown().await;
    }
}
