//! Del tas de estado en la forma del contrato IPC (SAD §4.6).
//!
//! El dominio ya expone la ruta textual de cada parámetro
//! ([`StateOp::path`](evo_domain::StateOp::path)); aquí se combina esa ruta con
//! el valor para producir el `{ "path": ..., "v": ... }` que viaja por el cable:
//!
//! ```jsonc
//! { "path": "ch/0/gain", "v": 31.5 }
//! ```
//!
//! Se usa una ruta *string* en lugar del enum tipado para que un cliente pueda
//! ignorar rutas que no conoce (compatibilidad hacia adelante) y para mantener
//! un único formato legible en el protocolo.

use evo_domain::{SmartGainPhase, StateOp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Un cambio puntual de estado con ruta textual y valor JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchOp {
    /// Ruta del parámetro, p. ej. `"ch/0/gain"`, `"out/1/mix"`, `"loopback"`.
    pub path: String,
    /// Valor nuevo del parámetro.
    ///
    /// Es escalar (`31.5`, `true`, `"listening"`) salvo los parámetros
    /// compuestos: `out/{n}/mix` lleva un objeto `{ "level_db": .., "pan": .. }`.
    pub v: Value,
}

impl PatchOp {
    /// Construye la operación IPC a partir de un delta del dominio.
    pub fn from_state_op(op: &StateOp) -> Self {
        let path = op.path();
        let v = match *op {
            StateOp::Gain { db, .. } => json!(db),
            StateOp::Mute { on, .. }
            | StateOp::Phantom { on, .. }
            | StateOp::Pad { on, .. }
            | StateOp::Instrument { on, .. }
            | StateOp::Loopback { on } => json!(on),
            StateOp::MonitorMix { level_db, pan, .. } => {
                json!({ "level_db": level_db, "pan": pan })
            }
            StateOp::SmartGain { phase } => json!(phase_str(phase)),
        };
        Self { path, v }
    }

    /// Recupera el delta del dominio, si la ruta y el valor son válidos.
    ///
    /// Es la operación inversa de [`PatchOp::from_state_op`] y el punto de
    /// entrada de los clientes que reaplican parches sobre una copia local del
    /// estado. Devuelve `None` ante una ruta desconocida o un valor de tipo
    /// incorrecto (el parche se ignora, no rompe la sesión).
    pub fn to_state_op(&self) -> Option<StateOp> {
        if let Some(rest) = self.path.strip_prefix("ch/") {
            let (ch, param) = rest.split_once('/')?;
            let ch: u8 = ch.parse().ok()?;
            return match param {
                "gain" => Some(StateOp::Gain {
                    ch,
                    db: self.v.as_f64()? as f32,
                }),
                "mute" => Some(StateOp::Mute {
                    ch,
                    on: self.v.as_bool()?,
                }),
                "phantom" => Some(StateOp::Phantom {
                    ch,
                    on: self.v.as_bool()?,
                }),
                "pad" => Some(StateOp::Pad {
                    ch,
                    on: self.v.as_bool()?,
                }),
                "instrument" => Some(StateOp::Instrument {
                    ch,
                    on: self.v.as_bool()?,
                }),
                _ => None,
            };
        }
        if let Some(rest) = self.path.strip_prefix("out/") {
            let (out, param) = rest.split_once('/')?;
            let out: u8 = out.parse().ok()?;
            if param != "mix" {
                return None;
            }
            return Some(StateOp::MonitorMix {
                out,
                level_db: self.v.get("level_db")?.as_f64()? as f32,
                pan: self.v.get("pan")?.as_f64()? as f32,
            });
        }
        match self.path.as_str() {
            "loopback" => Some(StateOp::Loopback {
                on: self.v.as_bool()?,
            }),
            "smartgain" => Some(StateOp::SmartGain {
                phase: parse_phase(self.v.as_str()?)?,
            }),
            _ => None,
        }
    }
}

fn phase_str(phase: SmartGainPhase) -> &'static str {
    match phase {
        SmartGainPhase::Idle => "idle",
        SmartGainPhase::Listening => "listening",
        SmartGainPhase::Calibrating => "calibrating",
        SmartGainPhase::Success => "success",
        SmartGainPhase::Error => "error",
        SmartGainPhase::Cancelled => "cancelled",
    }
}

fn parse_phase(s: &str) -> Option<SmartGainPhase> {
    Some(match s {
        "idle" => SmartGainPhase::Idle,
        "listening" => SmartGainPhase::Listening,
        "calibrating" => SmartGainPhase::Calibrating,
        "success" => SmartGainPhase::Success,
        "error" => SmartGainPhase::Error,
        "cancelled" => SmartGainPhase::Cancelled,
        _ => return None,
    })
}
