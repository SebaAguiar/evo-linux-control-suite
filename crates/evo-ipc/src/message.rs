//! Mensajes de control del WebSocket (SAD §4.6).
//!
//! El sobre lleva una etiqueta `t` (`type`) y el comando reutiliza
//! [`DeviceCommand`] del dominio: su propia etiqueta `op` se aplana dentro del
//! mensaje, así que el cable queda exactamente como lo documenta el SAD:
//!
//! ```jsonc
//! { "t": "cmd", "id": 42, "op": "set_gain", "ch": 0, "db": 31.5 }
//! ```
//!
//! Reutilizar el tipo del dominio evita una segunda definición de los comandos
//! que podría desincronizarse de la real.

use evo_domain::{
    CommandRejection, ConnectionState, DeviceCommand, DeviceProfile, DeviceState, StateEvent,
    StateSnapshot,
};
use serde::{Deserialize, Serialize};

use crate::PatchOp;

/// Mensaje de cliente a servidor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Ejecuta un comando. `id` lo elige el cliente para casar la respuesta
    /// ([`ServerMessage::Ack`] / [`ServerMessage::Err`]); es opaco para el
    /// servidor.
    Cmd {
        id: u64,
        #[serde(flatten)]
        cmd: DeviceCommand,
    },
    /// Pide una instantánea completa. El cliente lo usa cuando detecta un salto
    /// de `rev` o al recuperarse de una desconexión (SAD §2.4).
    Resync,
    /// Handshake de autenticación **reservado** para v1.x (SAD §8.4).
    ///
    /// En v1.0 no hay cuentas ni roles: cualquier cliente que alcanza el
    /// servidor tiene acceso completo, así que el servidor acepta y no hace nada.
    /// El tipo existe ya para que el contrato no cambie cuando lleguen los
    /// tokens/PIN.
    Auth { token: String },
}

impl ClientMessage {
    /// Atajo para construir un comando con `id`.
    pub fn command(id: u64, cmd: DeviceCommand) -> Self {
        Self::Cmd { id, cmd }
    }

    /// El comando contenido, si el mensaje es un [`ClientMessage::Cmd`].
    pub fn command_op(&self) -> Option<DeviceCommand> {
        match self {
            ClientMessage::Cmd { cmd, .. } => Some(*cmd),
            _ => None,
        }
    }
}

/// Mensaje de servidor a cliente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Primer mensaje tras el *upgrade*: anuncia [`PROTO_VERSION`].
    ///
    /// [`PROTO_VERSION`]: crate::PROTO_VERSION
    Hello { proto_version: u16 },
    /// Instantánea completa versionada (al conectar o tras un resync).
    Snapshot {
        rev: u64,
        profile: DeviceProfile,
        state: DeviceState,
    },
    /// Delta versionado aplicado al estado.
    Patch { rev: u64, ops: Vec<PatchOp> },
    /// Confirmación de que un comando fue **aceptado** (no que se haya aplicado
    /// ya: el efecto llega como [`ServerMessage::Patch`], SAD §4.6).
    Ack { id: u64, ok: bool },
    /// Fallo al aceptar un comando, con motivo tipado.
    Err {
        id: u64,
        code: ErrorCode,
        msg: String,
    },
    /// Cambio en el estado de conexión del dispositivo.
    ///
    /// Es una **extensión aditiva** del contrato de §4.6: el SAD pide reflejar
    /// la conexión en la UI (estados `Enumerating`/`Connected`/…) pero no fija un
    /// mensaje, y el estado de conexión es efímero (no vive en el `StateStore`,
    /// SAD §2.4), así que no cabe en un [`PatchOp`]. Un cliente que no lo conozca
    /// lo ignora.
    Connection { state: ConnectionState },
}

impl ServerMessage {
    /// Construye el [`ServerMessage::Hello`] de apertura.
    pub fn hello() -> Self {
        Self::Hello {
            proto_version: crate::PROTO_VERSION,
        }
    }

    /// Construye un [`ServerMessage::Snapshot`] a partir de una instantánea.
    pub fn snapshot(snapshot: StateSnapshot) -> Self {
        Self::Snapshot {
            rev: snapshot.rev,
            profile: snapshot.profile,
            state: snapshot.state,
        }
    }

    /// Traduce un delta del dominio al mensaje [`ServerMessage::Patch`].
    pub fn patch(event: &StateEvent) -> Self {
        Self::Patch {
            rev: event.rev,
            ops: event.ops.iter().map(PatchOp::from_state_op).collect(),
        }
    }

    /// Confirmación de un comando aceptado.
    pub fn ack(id: u64) -> Self {
        Self::Ack { id, ok: true }
    }

    /// Rechazo de un comando, con el [`ErrorCode`] derivado del motivo real.
    pub fn error(id: u64, rejection: &CommandRejection) -> Self {
        Self::Err {
            id,
            code: ErrorCode::from_rejection(rejection),
            msg: rejection.to_string(),
        }
    }

    /// Rechazo por causas que no son del dominio (mensaje malformado, límite de
    /// tasa…), con `id = 0` cuando no se pudo leer.
    pub fn protocol_error(id: u64, code: ErrorCode, msg: impl Into<String>) -> Self {
        Self::Err {
            id,
            code,
            msg: msg.into(),
        }
    }
}

/// Código de error estable del protocolo.
///
/// Es una taxonomía corta y cerrada (no el texto del error): el cliente decide
/// cómo mostrarlo o si reintentar sin parsear mensajes en español.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// El comando viola un invariante del dominio (índice fuera de rango, valor
    /// no finito).
    BadRequest,
    /// La [`SafetyPolicy`](evo_domain::SafetyPolicy) lo prohibió (p. ej. phantom
    /// bloqueado globalmente).
    Forbidden,
    /// Recurso ocupado de forma transitoria (SmartGain ya en curso).
    Busy,
    /// Rechazado por límite de tasa o de tamaño, o mensaje malformado.
    Rejected,
    /// Fallo interno del servidor.
    Internal,
}

impl ErrorCode {
    /// Clasifica el motivo de un rechazo del núcleo.
    pub fn from_rejection(rejection: &CommandRejection) -> Self {
        use evo_domain::SafetyError;
        match rejection {
            CommandRejection::Domain(_) => ErrorCode::BadRequest,
            CommandRejection::Safety(SafetyError::PhantomLocked) => ErrorCode::Forbidden,
            CommandRejection::Safety(SafetyError::SmartGainBusy) => ErrorCode::Busy,
            CommandRejection::Safety(_) => ErrorCode::Rejected,
        }
    }
}
