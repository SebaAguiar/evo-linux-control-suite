//! Tests de integración de `evo-remote` sobre un WebSocket **real** (misma pila
//! `tungstenite`), con `evo-mock` como dispositivo: ejercitan el contrato de
//! §4.6 de punta a punta sin hardware.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use evo_core::{CoreConfig, EvoCore};
use evo_domain::{DeviceCommand, MeterFrame, ModelId};
use evo_ipc::{ClientMessage, ServerMessage};
use evo_mock::MockDevice;
use evo_remote::{OriginPolicy, RemoteConfig, serve};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Arranca núcleo (sobre mock EVO 4) + servidor en un puerto efímero.
async fn start(mut config: RemoteConfig) -> (Arc<EvoCore>, evo_remote::RemoteHandle) {
    config.bind = SocketAddr::from(([127, 0, 0, 1], 0));
    config.meter_interval = Duration::from_millis(10);
    let core = Arc::new(EvoCore::spawn(
        Box::new(MockDevice::new(ModelId::Evo4)),
        CoreConfig::default(),
    ));
    let handle = serve(core.clone(), config)
        .await
        .expect("enlaza el servidor");
    (core, handle)
}

async fn connect(addr: SocketAddr) -> Ws {
    let (ws, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("conecta");
    ws
}

/// Lee el próximo mensaje de texto como [`ServerMessage`], saltando binarios.
async fn next_text(ws: &mut Ws) -> ServerMessage {
    loop {
        let msg = timeout(READ_TIMEOUT, ws.next())
            .await
            .expect("no llegó mensaje a tiempo")
            .expect("stream abierto")
            .expect("mensaje sin error");
        match msg {
            Message::Text(text) => return serde_json::from_str(&text).expect("JSON válido"),
            Message::Binary(_) => continue,
            Message::Close(_) => panic!("el servidor cerró la conexión"),
            _ => continue,
        }
    }
}

/// Lee mensajes de texto hasta que uno cumpla el predicado.
async fn next_until(ws: &mut Ws, predicate: impl Fn(&ServerMessage) -> bool) -> ServerMessage {
    for _ in 0..16 {
        let msg = next_text(ws).await;
        if predicate(&msg) {
            return msg;
        }
    }
    panic!("no llegó el mensaje esperado");
}

async fn send(ws: &mut Ws, msg: &ClientMessage) {
    let text = serde_json::to_string(msg).expect("serializa");
    ws.send(Message::Text(text.into())).await.expect("envía");
}

#[tokio::test]
async fn handshake_envia_hello_y_snapshot() {
    let (_core, handle) = start(RemoteConfig::default()).await;
    let mut ws = connect(handle.local_addr()).await;

    assert!(matches!(
        next_text(&mut ws).await,
        ServerMessage::Hello { proto_version } if proto_version == evo_ipc::PROTO_VERSION
    ));

    match next_text(&mut ws).await {
        ServerMessage::Snapshot {
            rev,
            profile,
            state,
        } => {
            assert_eq!(rev, 0);
            assert_eq!(profile.inputs, 2);
            assert_eq!(state.channels.len(), 2);
        }
        other => panic!("se esperaba snapshot, llegó {other:?}"),
    }

    handle.shutdown().await;
}

#[tokio::test]
async fn un_comando_se_ack_y_se_difunde_como_patch() {
    let (_core, handle) = start(RemoteConfig::default()).await;
    let mut ws = connect(handle.local_addr()).await;
    // Consume hello + snapshot inicial.
    next_text(&mut ws).await;
    next_text(&mut ws).await;

    send(
        &mut ws,
        &ClientMessage::command(1, DeviceCommand::SetGain { ch: 0, db: 12.0 }),
    )
    .await;

    let ack = next_until(&mut ws, |m| matches!(m, ServerMessage::Ack { .. })).await;
    assert_eq!(ack, ServerMessage::Ack { id: 1, ok: true });

    let patch = next_until(&mut ws, |m| matches!(m, ServerMessage::Patch { .. })).await;
    match patch {
        ServerMessage::Patch { rev, ops } => {
            assert!(rev >= 1);
            assert!(
                ops.iter().any(|op| op.path == "ch/0/gain"),
                "el patch debe incluir ch/0/gain: {ops:?}"
            );
        }
        other => panic!("se esperaba patch, llegó {other:?}"),
    }

    handle.shutdown().await;
}

#[tokio::test]
async fn un_canal_fuera_de_rango_devuelve_bad_request() {
    let (_core, handle) = start(RemoteConfig::default()).await;
    let mut ws = connect(handle.local_addr()).await;
    next_text(&mut ws).await;
    next_text(&mut ws).await;

    send(
        &mut ws,
        &ClientMessage::command(7, DeviceCommand::SetGain { ch: 99, db: 12.0 }),
    )
    .await;

    let err = next_until(&mut ws, |m| matches!(m, ServerMessage::Err { .. })).await;
    match err {
        ServerMessage::Err { id, code, .. } => {
            assert_eq!(id, 7);
            assert_eq!(code, evo_ipc::ErrorCode::BadRequest);
        }
        other => panic!("se esperaba err, llegó {other:?}"),
    }

    handle.shutdown().await;
}

#[tokio::test]
async fn el_phantom_bloqueado_devuelve_forbidden() {
    let (core, handle) = start(RemoteConfig::default()).await;
    core.set_phantom_lock(true).await.expect("bloquea phantom");

    let mut ws = connect(handle.local_addr()).await;
    next_text(&mut ws).await;
    next_text(&mut ws).await;

    send(
        &mut ws,
        &ClientMessage::command(9, DeviceCommand::SetPhantom { ch: 0, on: true }),
    )
    .await;

    let err = next_until(&mut ws, |m| matches!(m, ServerMessage::Err { .. })).await;
    match err {
        ServerMessage::Err { id, code, .. } => {
            assert_eq!(id, 9);
            assert_eq!(code, evo_ipc::ErrorCode::Forbidden);
        }
        other => panic!("se esperaba err, llegó {other:?}"),
    }

    handle.shutdown().await;
}

#[tokio::test]
async fn resync_devuelve_una_instantanea() {
    let (_core, handle) = start(RemoteConfig::default()).await;
    let mut ws = connect(handle.local_addr()).await;
    next_text(&mut ws).await; // hello
    next_text(&mut ws).await; // snapshot

    send(&mut ws, &ClientMessage::Resync).await;

    assert!(matches!(
        next_until(&mut ws, |m| matches!(m, ServerMessage::Snapshot { .. })).await,
        ServerMessage::Snapshot { .. }
    ));

    handle.shutdown().await;
}

#[tokio::test]
async fn un_mensaje_malformado_devuelve_error_sin_cerrar_la_sesion() {
    let (_core, handle) = start(RemoteConfig::default()).await;
    let mut ws = connect(handle.local_addr()).await;
    next_text(&mut ws).await;
    next_text(&mut ws).await;

    ws.send(Message::Text("esto no es json".into()))
        .await
        .expect("envía");

    let err = next_until(&mut ws, |m| matches!(m, ServerMessage::Err { .. })).await;
    assert!(matches!(err, ServerMessage::Err { .. }));

    // La sesión sigue viva: un comando válido posterior se confirma.
    send(&mut ws, &ClientMessage::Resync).await;
    assert!(matches!(
        next_until(&mut ws, |m| matches!(m, ServerMessage::Snapshot { .. })).await,
        ServerMessage::Snapshot { .. }
    ));

    handle.shutdown().await;
}

#[tokio::test]
async fn los_medidores_viajan_como_fotograma_binario() {
    let (core, handle) = start(RemoteConfig::default()).await;
    let mut ws = connect(handle.local_addr()).await;
    next_text(&mut ws).await; // hello
    next_text(&mut ws).await; // snapshot

    core.publish_meters(MeterFrame::from_db_fs(1, &[-3.0, -10.0]));

    // El próximo mensaje binario debe decodificar al fotograma publicado.
    let bytes = loop {
        let msg = timeout(READ_TIMEOUT, ws.next())
            .await
            .expect("no llegó el fotograma a tiempo")
            .expect("stream abierto")
            .expect("mensaje sin error");
        if let Message::Binary(bytes) = msg {
            break bytes;
        }
    };
    let frame = MeterFrame::decode(&bytes).expect("fotograma válido");
    assert_eq!(frame.seq, 1);
    assert_eq!(frame.channels(), 2);
    assert_eq!(frame.peak_db_fs(0), Some(-3.0));
    assert_eq!(frame.peak_db_fs(1), Some(-10.0));

    handle.shutdown().await;
}

#[tokio::test]
async fn un_origin_publico_es_rechazado_en_el_upgrade() {
    let (_core, handle) = start(RemoteConfig::default()).await;

    let mut request = format!("ws://{}/ws", handle.local_addr())
        .into_client_request()
        .expect("petición válida");
    request.headers_mut().insert(
        tokio_tungstenite::tungstenite::http::header::ORIGIN,
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("http://evil.example.com"),
    );

    let result = connect_async(request).await;
    assert!(
        result.is_err(),
        "un Origin público no debe poder conectarse"
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn con_allow_any_el_origin_publico_es_aceptado() {
    let config = RemoteConfig {
        origin: OriginPolicy::AllowAny,
        ..RemoteConfig::default()
    };
    let (_core, handle) = start(config).await;

    let mut request = format!("ws://{}/ws", handle.local_addr())
        .into_client_request()
        .expect("petición válida");
    request.headers_mut().insert(
        tokio_tungstenite::tungstenite::http::header::ORIGIN,
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("http://evil.example.com"),
    );

    let (mut ws, _) = connect_async(request).await.expect("se conecta");
    assert!(matches!(
        next_text(&mut ws).await,
        ServerMessage::Hello { .. }
    ));

    handle.shutdown().await;
}
