//! Sesión de WebSocket: *handshake*, comandos y difusión de estado (SAD §4.6).
//!
//! Cada conexión tiene su propio canal de salida ([`mpsc`]) que alimentan tres
//! tareas de reenvío (estado, conexión, medidores). Separarlas en tareas evita
//! que el *broadcast* del núcleo quede bloqueado por un cliente lento: si su
//! socket no drena, solo se llena *su* canal, no el bus compartido.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use evo_core::{CoreError, EvoCore};
use evo_domain::DeviceCommand;
use evo_ipc::{ClientMessage, ErrorCode, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc};

use crate::{AppState, peer};

/// Maneja el *upgrade*: valida origen y tamaño antes de ceder el socket.
pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !peer::is_allowed_peer(peer_addr.ip(), state.config.allow_public_peers) {
        tracing::warn!(peer = %peer_addr, "control remoto: IP de origen no permitida");
        return (StatusCode::FORBIDDEN, "origen no permitido").into_response();
    }

    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !peer::is_allowed_origin(origin, state.config.origin, &state.config.allowed_origins) {
        tracing::warn!(?origin, "control remoto: Origin no permitido");
        return (StatusCode::FORBIDDEN, "origin no permitido").into_response();
    }

    let max = state.config.max_message_bytes;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| session(socket, state))
}

/// Ciclo de vida de una conexión.
async fn session(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(64);

    // Un único escritor: el resto de tareas publican en `tx`.
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Handshake y estado inicial (SAD §4.6): primero la versión, luego el estado.
    send(&tx, &ServerMessage::hello()).await;
    send(&tx, &ServerMessage::snapshot(state.core.snapshot())).await;

    let forwarders = [
        tokio::spawn(forward_state(state.core.clone(), tx.clone())),
        tokio::spawn(forward_connection(state.core.clone(), tx.clone())),
        tokio::spawn(forward_meters(
            state.core.clone(),
            state.config.meter_interval,
            tx.clone(),
        )),
    ];

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => handle_text(&state, &tx, &text).await,
            Message::Close(_) => break,
            // Tungstenite responde a los `ping` por nosotros; el canal binario es
            // servidor → cliente (medidores), el cliente no lo usa.
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    for task in forwarders {
        task.abort();
    }
    drop(tx);
    let _ = writer.await;
    tracing::debug!("sesión de control remoto cerrada");
}

/// Procesa un mensaje de control del cliente.
async fn handle_text(state: &AppState, tx: &mpsc::Sender<Message>, text: &str) {
    let msg = match serde_json::from_str::<ClientMessage>(text) {
        Ok(msg) => msg,
        Err(err) => {
            tracing::debug!(error = %err, "control remoto: mensaje inválido");
            send(
                tx,
                &ServerMessage::protocol_error(0, ErrorCode::Rejected, "mensaje inválido"),
            )
            .await;
            return;
        }
    };

    match msg {
        ClientMessage::Cmd { id, cmd } => spawn_command(state.clone(), tx.clone(), id, cmd),
        ClientMessage::Resync => {
            let _ = state.core.resync().await;
            send(tx, &ServerMessage::snapshot(state.core.snapshot())).await;
        }
        // v1.0 no tiene cuentas ni roles: el handshake se acepta y se ignora
        // (SAD §8.4). El contrato lo reserva para cuando lleguen los tokens.
        ClientMessage::Auth { .. } => {
            tracing::debug!("control remoto: handshake auth ignorado (v1.0 sin roles)");
        }
    }
}

/// Ejecuta un comando en el núcleo y responde `ack`/`err`.
///
/// Se lanza en su propia tarea para no bloquear la lectura del socket mientras
/// el actor aplica el comando.
fn spawn_command(state: AppState, tx: mpsc::Sender<Message>, id: u64, cmd: DeviceCommand) {
    tokio::spawn(async move {
        let reply = match state.core.submit(cmd).await {
            Ok(()) => ServerMessage::ack(id),
            Err(CoreError::Rejected(rejection)) => ServerMessage::error(id, &rejection),
            Err(CoreError::Shutdown) => {
                ServerMessage::protocol_error(id, ErrorCode::Internal, "el núcleo se está apagando")
            }
        };
        send(&tx, &reply).await;
    });
}

/// Reenvía los deltas de estado como `patch`.
async fn forward_state(core: Arc<EvoCore>, tx: mpsc::Sender<Message>) {
    let mut rx = core.subscribe_state();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if !send(&tx, &ServerMessage::patch(&event)).await {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                // El cliente no drenó a tiempo: en vez de perder deltas en
                // silencio, se le manda una instantánea completa (SAD §2.4).
                tracing::warn!(
                    skipped,
                    "cliente lento: se envía instantánea de resincronización"
                );
                if !send(&tx, &ServerMessage::snapshot(core.snapshot())).await {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// Reenvía los cambios de conexión del dispositivo.
async fn forward_connection(core: Arc<EvoCore>, tx: mpsc::Sender<Message>) {
    let mut rx = core.subscribe_connection();
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        let state = *rx.borrow_and_update();
        if !send(&tx, &ServerMessage::Connection { state }).await {
            break;
        }
    }
}

/// Reenvía fotogramas de medidores binarios, con *throttling* por cliente.
async fn forward_meters(core: Arc<EvoCore>, interval: Duration, tx: mpsc::Sender<Message>) {
    let mut rx = core.subscribe_meters();
    loop {
        // El `watch` solo despierta cuando se publica un fotograma nuevo.
        if rx.changed().await.is_err() {
            break;
        }
        // Colapsa ráfagas: como el `watch` conserva solo el último fotograma,
        // dormir aquí equivale a enviar como mucho uno por intervalo (SAD §2.2).
        tokio::time::sleep(interval).await;
        let frame = *rx.borrow_and_update();
        if tx
            .send(Message::Binary(frame.encode().into()))
            .await
            .is_err()
        {
            break;
        }
    }
}

/// Serializa y encola un mensaje de control.
///
/// Devuelve `false` solo si el canal de salida está cerrado (la sesión terminó).
async fn send(tx: &mpsc::Sender<Message>, msg: &ServerMessage) -> bool {
    match serde_json::to_string(msg) {
        Ok(text) => tx.send(Message::Text(text.into())).await.is_ok(),
        Err(err) => {
            // No debería ocurrir con estado validado (sin NaN); si pasa, se
            // registra y se sigue, sin tumbar la sesión por un mensaje.
            tracing::warn!(error = %err, "control remoto: no se pudo serializar un mensaje");
            true
        }
    }
}
