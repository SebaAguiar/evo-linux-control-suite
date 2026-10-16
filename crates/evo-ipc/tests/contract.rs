//! El contrato de `evo-ipc` debe casar *exactamente* con los ejemplos del SAD
//! §4.6: estos tests fijan la forma del cable (nombres de campo y estructura),
//! no solo que serialice sin error.

use evo_domain::{
    CommandRejection, ConnectionState, DeviceCommand, DeviceProfile, DeviceState, SafetyError,
    SmartGainPhase, StateEvent, StateOp, StateSnapshot,
};
use evo_ipc::{ClientMessage, ErrorCode, PROTO_VERSION, PatchOp, ServerMessage};
use serde_json::json;

fn round_trip_client(msg: &ClientMessage) -> ClientMessage {
    let text = serde_json::to_string(msg).expect("serializa");
    serde_json::from_str(&text).expect("deserializa")
}

#[test]
fn cmd_matches_sad_example() {
    let msg = ClientMessage::command(42, DeviceCommand::SetGain { ch: 0, db: 31.5 });
    let value = serde_json::to_value(&msg).expect("serializa");

    // Ejemplo literal del SAD §4.6.
    assert_eq!(
        value,
        json!({ "t": "cmd", "id": 42, "op": "set_gain", "ch": 0, "db": 31.5 })
    );
    assert_eq!(round_trip_client(&msg), msg);
}

#[test]
fn trigger_smartgain_matches_sad_example() {
    let msg = ClientMessage::command(43, DeviceCommand::TriggerSmartGain);
    let value = serde_json::to_value(&msg).expect("serializa");
    assert_eq!(
        value,
        json!({ "t": "cmd", "id": 43, "op": "trigger_smartgain" })
    );
    assert_eq!(round_trip_client(&msg), msg);
}

#[test]
fn resync_round_trip() {
    let msg = ClientMessage::Resync;
    assert_eq!(
        serde_json::to_value(&msg).unwrap(),
        json!({ "t": "resync" })
    );
    assert_eq!(round_trip_client(&msg), msg);
}

#[test]
fn auth_handshake_is_reserved_and_round_trips() {
    // El contrato reserva `auth` para v1.x (SAD §8.4); el tipo ya existe.
    let msg = ClientMessage::Auth {
        token: "abc".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&msg).unwrap(),
        json!({ "t": "auth", "token": "abc" })
    );
    assert_eq!(round_trip_client(&msg), msg);
}

#[test]
fn every_command_op_round_trips() {
    let ops = [
        DeviceCommand::SetGain { ch: 1, db: -3.5 },
        DeviceCommand::SetMute { ch: 1, on: true },
        DeviceCommand::SetPhantom { ch: 0, on: true },
        DeviceCommand::SetMonitorMix {
            out: 1,
            level_db: -6.0,
            pan: 0.25,
        },
        DeviceCommand::SetLoopback { on: true },
        DeviceCommand::TriggerSmartGain,
        DeviceCommand::CancelSmartGain,
        DeviceCommand::SetPad { ch: 2, on: true },
        DeviceCommand::SetInstrument { ch: 3, on: true },
    ];
    for (i, op) in ops.into_iter().enumerate() {
        let msg = ClientMessage::command(i as u64, op);
        assert_eq!(round_trip_client(&msg), msg, "op {op:?}");
        assert_eq!(msg.command_op(), Some(op));
    }
}

#[test]
fn hello_announces_proto_version() {
    let value = serde_json::to_value(ServerMessage::hello()).unwrap();
    assert_eq!(
        value,
        json!({ "t": "hello", "proto_version": PROTO_VERSION })
    );
}

#[test]
fn snapshot_message_carries_profile_and_state() {
    let profile = DeviceProfile::evo4();
    let state = DeviceState::initial(&profile);
    let snapshot = StateSnapshot {
        rev: 7,
        profile: profile.clone(),
        state: state.clone(),
    };

    let value = serde_json::to_value(ServerMessage::snapshot(snapshot)).unwrap();
    assert_eq!(value["t"], json!("snapshot"));
    assert_eq!(value["rev"], json!(7));
    assert_eq!(value["profile"]["inputs"], json!(2));
    assert_eq!(value["state"]["channels"].as_array().unwrap().len(), 2);
}

#[test]
fn patch_message_matches_sad_example() {
    let event = StateEvent {
        rev: 1205,
        ops: vec![StateOp::Gain { ch: 0, db: 31.5 }],
    };
    let value = serde_json::to_value(ServerMessage::patch(&event)).unwrap();
    assert_eq!(
        value,
        json!({
            "t": "patch",
            "rev": 1205,
            "ops": [ { "path": "ch/0/gain", "v": 31.5 } ]
        })
    );
}

#[test]
fn patch_op_shape_and_reverse() {
    let op = PatchOp::from_state_op(&StateOp::Gain { ch: 0, db: 31.5 });
    assert_eq!(
        serde_json::to_value(&op).unwrap(),
        json!({ "path": "ch/0/gain", "v": 31.5 })
    );
    assert_eq!(op.to_state_op(), Some(StateOp::Gain { ch: 0, db: 31.5 }));
}

#[test]
fn every_state_op_round_trips_through_patch_op() {
    let ops = [
        StateOp::Gain { ch: 2, db: 12.0 },
        StateOp::Mute { ch: 0, on: true },
        StateOp::Phantom { ch: 1, on: false },
        StateOp::Pad { ch: 3, on: true },
        StateOp::Instrument { ch: 3, on: true },
        StateOp::MonitorMix {
            out: 1,
            level_db: -4.5,
            pan: -0.75,
        },
        StateOp::Loopback { on: true },
        StateOp::SmartGain {
            phase: SmartGainPhase::Calibrating,
        },
        StateOp::SmartGain {
            phase: SmartGainPhase::Success,
        },
    ];
    for op in ops {
        let patch = PatchOp::from_state_op(&op);
        assert_eq!(patch.to_state_op(), Some(op), "{op:?}");
    }
}

#[test]
fn unknown_or_malformed_path_is_ignored() {
    let unknown = PatchOp {
        path: "future/thing".to_string(),
        v: json!(1),
    };
    assert_eq!(unknown.to_state_op(), None);

    let wrong_type = PatchOp {
        path: "ch/0/mute".to_string(),
        v: json!(31.5),
    };
    assert_eq!(wrong_type.to_state_op(), None);

    let mix_missing_pan = PatchOp {
        path: "out/0/mix".to_string(),
        v: json!({ "level_db": -3.0 }),
    };
    assert_eq!(mix_missing_pan.to_state_op(), None);
}

#[test]
fn ack_and_error_shapes() {
    assert_eq!(
        serde_json::to_value(ServerMessage::ack(42)).unwrap(),
        json!({ "t": "ack", "id": 42, "ok": true })
    );

    let forbidden = CommandRejection::Safety(SafetyError::PhantomLocked);
    let value = serde_json::to_value(ServerMessage::error(43, &forbidden)).unwrap();
    assert_eq!(value["t"], json!("err"));
    assert_eq!(value["id"], json!(43));
    assert_eq!(value["code"], json!("forbidden"));
}

#[test]
fn error_codes_classify_rejections() {
    use evo_domain::DomainError;
    assert_eq!(
        ErrorCode::from_rejection(&CommandRejection::Domain(DomainError::ChannelOutOfRange {
            ch: 9,
            channels: 2
        })),
        ErrorCode::BadRequest
    );
    assert_eq!(
        ErrorCode::from_rejection(&CommandRejection::Safety(SafetyError::PhantomLocked)),
        ErrorCode::Forbidden
    );
    assert_eq!(
        ErrorCode::from_rejection(&CommandRejection::Safety(SafetyError::SmartGainBusy)),
        ErrorCode::Busy
    );
    assert_eq!(
        ErrorCode::from_rejection(&CommandRejection::Safety(SafetyError::PhantomCooldown {
            remaining_ms: 100
        })),
        ErrorCode::Rejected
    );
}

#[test]
fn connection_message_is_additive_extension() {
    let value = serde_json::to_value(ServerMessage::Connection {
        state: ConnectionState::Enumerating,
    })
    .unwrap();
    assert_eq!(value, json!({ "t": "connection", "state": "enumerating" }));
}
