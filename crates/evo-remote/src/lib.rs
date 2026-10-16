//! Adaptador de red del control remoto: servidor HTTP + WebSocket (SAD §4.5).
//!
//! Publica el [núcleo](evo_core::EvoCore) al cliente móvil por el contrato
//! compartido de [`evo_ipc`]. Es un adaptador *de entrada*: traduce mensajes del
//! cable a comandos del dominio y deltas del dominio a mensajes del cable, sin
//! lógica de negocio propia —la `SafetyPolicy` vive en el núcleo y se aplica a
//! **todos** los orígenes (SAD §8.4).
//!
//! # Por qué un servidor de biblioteca y no un binario
//!
//! El binario que lo arranca es `evo-daemon` (SAD §8.3); mantener `evo-remote`
//! como biblioteca permite montarlo en tests con un [`EvoCore`] sobre
//! [`evo_mock`](https://docs.rs/evo-mock), sin hardware ni red externa.
//!
//! # Qué cubre y qué difiere
//!
//! - **Cubre:** *upgrade* con filtros de origen, *handshake* (`hello` +
//!   `snapshot`), comandos (`cmd` → `ack`/`err`), difusión de deltas (`patch`),
//!   cambios de conexión, `resync`, reenvío de medidores binarios con *throttling*
//!   por cliente y límites de tamaño de mensaje.
//! - **Difiere:** el *bundle* estático del cliente móvil (`apps/remote-web`), el
//!   descubrimiento mDNS y el reenlazado al cambiar de IP de LAN. Requieren el
//!   frontend o la detección de interfaces, y no cambian este contrato. La ruta
//!   `/` sirve un marcador hasta que exista el *bundle*.

pub mod peer;
mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::response::Html;
use axum::routing::get;
use evo_core::EvoCore;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use peer::{OriginPolicy, is_allowed_origin, is_allowed_peer};

/// Estado compartido por todas las sesiones.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) core: Arc<EvoCore>,
    pub(crate) config: Arc<RemoteConfig>,
}

/// Ajustes del servidor remoto.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// Dirección de escucha. Por defecto loopback; el daemon la fija a la IP de
    /// la interfaz LAN elegida más loopback (SAD §8.4, nunca `0.0.0.0`).
    pub bind: SocketAddr,
    /// Si `true`, no se filtran las IP de origen públicas. **No** es el valor por
    /// defecto (SAD §8.4).
    pub allow_public_peers: bool,
    /// Política de validación de `Origin`.
    pub origin: OriginPolicy,
    /// Orígenes completos permitidos además de la heurística de `SameHost`
    /// (para nombres mDNS como `http://evo-suite.local:8080`).
    pub allowed_origins: Vec<String>,
    /// Intervalo mínimo entre fotogramas de medidores enviados a un cliente
    /// (*throttling* por cliente, SAD §2.2). El `watch` conserva solo el último
    /// fotograma, así que las ráfagas se colapsan a ~1 por intervalo.
    pub meter_interval: Duration,
    /// Tamaño máximo de mensaje entrante aceptado por el WebSocket.
    pub max_message_bytes: usize,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            allow_public_peers: false,
            origin: OriginPolicy::SameHost,
            allowed_origins: Vec::new(),
            meter_interval: Duration::from_millis(33),
            max_message_bytes: 64 * 1024,
        }
    }
}

/// Servidor remoto en ejecución.
pub struct RemoteHandle {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl RemoteHandle {
    /// Dirección real de escucha (útil cuando se pidió el puerto 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Apaga el servidor de forma ordenada y espera a que termine.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}

/// Construye el router HTTP/WebSocket montado sobre un núcleo.
///
/// Separado de [`serve`] para poder montarlo en tests o componerlo con otras
/// rutas sin abrir un puerto.
pub fn router(core: Arc<EvoCore>, config: RemoteConfig) -> Router {
    let state = AppState {
        core,
        config: Arc::new(config),
    };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(index))
        .route("/ws", get(ws::upgrade))
        .with_state(state)
}

/// Enlaza `config.bind` y sirve hasta que se llame a
/// [`RemoteHandle::shutdown`].
pub async fn serve(core: Arc<EvoCore>, config: RemoteConfig) -> std::io::Result<RemoteHandle> {
    let listener = TcpListener::bind(config.bind).await?;
    let addr = listener.local_addr()?;
    let app = router(core, config).into_make_service_with_connect_info::<SocketAddr>();

    let (tx, rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        let shutdown = async {
            let _ = rx.await;
        };
        if let Err(err) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!(error = %err, "el servidor remoto terminó con error");
        }
    });

    Ok(RemoteHandle {
        addr,
        shutdown: Some(tx),
        join,
    })
}

async fn healthz() -> &'static str {
    "ok"
}

async fn index() -> Html<&'static str> {
    Html(INDEX_PLACEHOLDER)
}

/// Página provisional de `/` hasta que exista el *bundle* de `apps/remote-web`.
const INDEX_PLACEHOLDER: &str = "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\">\
<title>EVO Control</title></head><body><h1>EVO Control</h1>\
<p>El cliente móvil todavía no está empaquetado. Conéctate por WebSocket en <code>/ws</code> \
con el contrato de <code>evo-ipc</code>.</p></body></html>";
