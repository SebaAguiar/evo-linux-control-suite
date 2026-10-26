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
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
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
    /// Dirección del listener de LAN, si se enlazó en paralelo.
    pub lan_addr: Option<SocketAddr>,
    /// Tamaño máximo de mensaje entrante aceptado por el WebSocket.
    pub max_message_bytes: usize,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            lan_addr: None,
            allow_public_peers: false,
            origin: OriginPolicy::SameHost,
            allowed_origins: Vec::new(),
            meter_interval: Duration::from_millis(33),
            max_message_bytes: 64 * 1024,
        }
    }
}

/// Parámetros de consulta opcionales para `/api/qr.svg`.
#[derive(Debug, serde::Deserialize)]
pub struct QrQuery {
    pub url: Option<String>,
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
        .route("/api/info", get(api_info))
        .route("/api/qr.svg", get(api_qr_svg))
        .route("/", get(index))
        .route("/ws", get(ws::upgrade))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, cors_headers))
}

/// Añade `Access-Control-Allow-Origin` a los orígenes que la política acepta.
///
/// El cliente de escritorio (Tauri) sirve su UI desde `tauri.localhost`, así que
/// su `fetch` a `/api/info` es *cross-origin*: sin esta cabecera el webview
/// bloquea la respuesta y la app cae al fallback de loopback aunque haya LAN.
async fn cors_headers(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let allowed = origin.as_deref().is_some_and(|origin| {
        is_allowed_origin(
            Some(origin),
            state.config.origin,
            &state.config.allowed_origins,
        )
    });

    let origin = origin.filter(|_| allowed);

    if req.method() == Method::OPTIONS {
        return with_cors(StatusCode::NO_CONTENT.into_response(), origin.as_deref());
    }

    with_cors(next.run(req).await, origin.as_deref())
}

/// Añade las cabeceras CORS para el `Origin` ya validado.
fn with_cors(mut res: Response, origin: Option<&str>) -> Response {
    let Some(origin) = origin else {
        return res;
    };
    let Ok(value) = HeaderValue::from_str(origin) else {
        return res;
    };
    let headers = res.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    res
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

/// Resuelve dinámicamente la IPv4 local activa consultando la tabla de
/// enrutamiento del kernel (vía UDP efímero) o enumerando interfaces físicas.
/// Refleja cambios de DHCP o cambio de red Wi-Fi/LAN al instante.
fn resolve_lan_ip() -> Option<std::net::Ipv4Addr> {
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("1.1.1.1:80").is_ok() {
            if let Ok(std::net::SocketAddr::V4(addr)) = socket.local_addr() {
                let ip = *addr.ip();
                if !ip.is_loopback() && !ip.is_unspecified() {
                    return Some(ip);
                }
            }
        }
    }

    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in &ifaces {
            let n = &iface.name;
            if n.starts_with("wl") || n.starts_with("en") || n.starts_with("eth") {
                if let if_addrs::IfAddr::V4(ref v4) = iface.addr {
                    if !v4.ip.is_loopback() && !v4.ip.is_unspecified() && !v4.ip.is_link_local() {
                        return Some(v4.ip);
                    }
                }
            }
        }

        for iface in ifaces {
            let n = &iface.name;
            if !n.starts_with("docker")
                && !n.starts_with("virbr")
                && !n.starts_with("veth")
                && !n.starts_with("br-")
            {
                if let if_addrs::IfAddr::V4(v4) = iface.addr {
                    if !v4.ip.is_loopback() && !v4.ip.is_unspecified() && !v4.ip.is_link_local() {
                        return Some(v4.ip);
                    }
                }
            }
        }
    }
    None
}

fn resolve_lan_url(config: &RemoteConfig) -> Option<String> {
    let port = config.bind.port();

    if let Some(ip) = resolve_lan_ip() {
        return Some(format!("http://{ip}:{port}"));
    }

    if let Some(addr) = config.lan_addr {
        return Some(format!("http://{}:{port}", addr.ip()));
    }

    None
}

async fn api_info(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::Json<serde_json::Value> {
    let port = state.config.bind.port();
    let lan_url = resolve_lan_url(&state.config);
    let loopback_url = format!("http://127.0.0.1:{port}");

    axum::Json(serde_json::json!({
        "loopback_url": loopback_url,
        "lan_url": lan_url,
        "port": port,
        "server_enabled": state.config.lan_addr.is_some(),
    }))
}

async fn api_qr_svg(
    axum::extract::Query(query): axum::extract::Query<QrQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let target_url = query.url.or_else(|| resolve_lan_url(&state.config)).unwrap_or_else(|| {
        format!("http://127.0.0.1:{}", state.config.bind.port())
    });

    match qrcode::QrCode::new(target_url.as_bytes()) {
        Ok(code) => {
            let svg_string = code
                .render::<qrcode::render::svg::Color>()
                .min_dimensions(260, 260)
                .dark_color(qrcode::render::svg::Color("#141518"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build();

            (
                axum::http::StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
                    (axum::http::header::CACHE_CONTROL, "no-cache"),
                ],
                svg_string,
            )
                .into_response()
        }
        Err(_) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [
                (axum::http::header::CONTENT_TYPE, "text/plain"),
                (axum::http::header::CACHE_CONTROL, "no-cache"),
            ],
            "error generando código QR".to_string(),
        )
            .into_response(),
    }
}

async fn index() -> Html<std::borrow::Cow<'static, str>> {
    let candidate_paths = [
        "apps/remote-web/public/index.html",
        "/home/seba/Documentos/trabajo/arsian/evo-linux-control-suite/apps/remote-web/public/index.html",
    ];
    for path in candidate_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            return Html(std::borrow::Cow::Owned(content));
        }
    }
    Html(std::borrow::Cow::Borrowed(INDEX_HTML))
}

/// Cliente web estático para control remoto (`apps/remote-web/public/index.html`).
const INDEX_HTML: &str = include_str!("../../../apps/remote-web/public/index.html");

#[cfg(test)]
mod tests {
    use super::*;
    use evo_core::CoreConfig;
    use evo_domain::ModelId;
    use evo_mock::MockDevice;

    #[tokio::test]
    async fn api_info_devuelve_urls_esperadas() {
        let core = Arc::new(EvoCore::spawn(
            Box::new(MockDevice::new(ModelId::Evo4)),
            CoreConfig::default(),
        ));
        let config = RemoteConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            lan_addr: Some(SocketAddr::from(([192, 168, 1, 50], 8080))),
            ..RemoteConfig::default()
        };
        let info = api_info(axum::extract::State(AppState {
            core,
            config: Arc::new(config),
        }))
        .await;

        let val = info.0;
        assert_eq!(val["loopback_url"], "http://127.0.0.1:8080");
        assert!(val["lan_url"].is_string());
        let lan_str = val["lan_url"].as_str().unwrap();
        assert!(lan_str.starts_with("http://") && lan_str.ends_with(":8080"));
        assert_eq!(val["port"], 8080);
    }

    #[tokio::test]
    async fn api_qr_svg_genera_svg_valido() {
        let core = Arc::new(EvoCore::spawn(
            Box::new(MockDevice::new(ModelId::Evo4)),
            CoreConfig::default(),
        ));
        let config = RemoteConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            lan_addr: Some(SocketAddr::from(([192, 168, 1, 50], 8080))),
            ..RemoteConfig::default()
        };
        let res = api_qr_svg(
            axum::extract::Query(QrQuery { url: None }),
            axum::extract::State(AppState {
                core,
                config: Arc::new(config),
            }),
        )
        .await;

        assert_eq!(res.status(), axum::http::StatusCode::OK);
        assert_eq!(
            res.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
            "image/svg+xml"
        );
    }

    #[tokio::test]
    async fn api_info_marca_el_servidor_remoto_como_deshabilitado() {
        let core = Arc::new(EvoCore::spawn(
            Box::new(MockDevice::new(ModelId::Evo4)),
            CoreConfig::default(),
        ));
        let config = RemoteConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            lan_addr: None,
            ..RemoteConfig::default()
        };
        let info = api_info(axum::extract::State(AppState {
            core,
            config: Arc::new(config),
        }))
        .await;

        assert_eq!(info.0["server_enabled"], false);
    }

    fn app_para_cors() -> Router {
        let core = Arc::new(EvoCore::spawn(
            Box::new(MockDevice::new(ModelId::Evo4)),
            CoreConfig::default(),
        ));
        router(core, RemoteConfig::default())
    }

    fn pedido(origin: &str) -> Request {
        Request::builder()
            .uri("/api/info")
            .header(header::ORIGIN, origin)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn cors_permite_origenes_locales_y_rechaza_publicos() {
        use tower::ServiceExt;

        let respuesta = app_para_cors()
            .oneshot(pedido("http://tauri.localhost"))
            .await
            .unwrap();
        assert_eq!(
            respuesta
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "http://tauri.localhost"
        );

        let rechazada = app_para_cors()
            .oneshot(pedido("http://evil.example.com"))
            .await
            .unwrap();
        assert!(
            rechazada
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
    }

    #[tokio::test]
    async fn cors_responde_al_preflight() {
        use tower::ServiceExt;

        let respuesta = app_para_cors()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/api/info")
                    .header(header::ORIGIN, "http://tauri.localhost")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(respuesta.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            respuesta
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "http://tauri.localhost"
        );
    }
}
