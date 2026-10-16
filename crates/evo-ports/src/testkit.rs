//! Suite de contrato para implementaciones de [`crate::EvoDevice`].
//!
//! Es la pieza que hace real la promesa del SAD §2.5: *"una suite genérica se
//! ejecuta contra `evo-mock` parametrizado por modelo y contra cada driver real"*.
//! Así, cuando en la Fase 3 aparezca el EVO 8/16, se valida contra el mismo
//! contrato sin escribir tests nuevos.
//!
//! Uso desde los tests de un adaptador:
//!
//! ```no_run
//! # use evo_ports::testkit::assert_device_contract;
//! # fn demo(device: &mut dyn evo_ports::EvoDevice) {
//! assert_device_contract(device);
//! # }
//! ```

use crate::EvoDevice;
use evo_domain::{Capabilities, DeviceCommand, DeviceEvent, MAX_CHANNELS, MAX_OUTPUTS};

/// Una violación del contrato de [`EvoDevice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    /// Nombre del check que falló.
    pub check: &'static str,
    /// Detalle de lo observado.
    pub detail: String,
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.check, self.detail)
    }
}

/// Ejecuta el contrato y devuelve **todas** las violaciones encontradas.
pub fn check_device_contract(device: &mut dyn EvoDevice) -> Vec<ContractViolation> {
    let mut v = Vec::new();
    let profile = device.profile().clone();

    // --- Perfil coherente y acotado -------------------------------------
    if profile.inputs == 0 || profile.outputs == 0 {
        v.push(ContractViolation {
            check: "profile_nonzero",
            detail: format!("inputs={} outputs={}", profile.inputs, profile.outputs),
        });
    }
    if profile.inputs as usize > MAX_CHANNELS || profile.outputs as usize > MAX_OUTPUTS {
        v.push(ContractViolation {
            check: "profile_within_capacity",
            detail: format!(
                "inputs={} (max {MAX_CHANNELS}), outputs={} (max {MAX_OUTPUTS})",
                profile.inputs, profile.outputs
            ),
        });
    }
    if !matches!(
        profile
            .gain_range_db
            .min_db
            .partial_cmp(&profile.gain_range_db.max_db),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    ) {
        v.push(ContractViolation {
            check: "profile_gain_range",
            detail: format!(
                "rango de ganancia no ordenado (¿NaN?): min={} max={}",
                profile.gain_range_db.min_db, profile.gain_range_db.max_db
            ),
        });
    }

    // --- read_state, si está soportado, debe ser consistente con el perfil --
    match device.read_state() {
        Ok(state) => {
            if state.channels.len() != profile.inputs as usize {
                v.push(ContractViolation {
                    check: "read_state_channels",
                    detail: format!(
                        "{} canales, el perfil declara {}",
                        state.channels.len(),
                        profile.inputs
                    ),
                });
            }
            if state.mixes.len() != profile.outputs as usize {
                v.push(ContractViolation {
                    check: "read_state_outputs",
                    detail: format!(
                        "{} salidas, el perfil declara {}",
                        state.mixes.len(),
                        profile.outputs
                    ),
                });
            }
        }
        Err(evo_domain::DeviceError::Unsupported(_)) => {}
        Err(other) => v.push(ContractViolation {
            check: "read_state_error",
            detail: format!("error inesperado: {other}"),
        }),
    }

    let mid_db = (profile.gain_range_db.min_db + profile.gain_range_db.max_db) / 2.0;

    // --- Ganancia: comando dentro de rango se refleja tal cual ------------
    match device.apply(DeviceCommand::SetGain { ch: 0, db: mid_db }) {
        Ok(events) => {
            check_events(&mut v, &profile, "set_gain", &events);
            let ok = events.iter().any(|e| {
                matches!(e, DeviceEvent::GainChanged { ch, db }
                    if *ch == 0 && (*db - mid_db).abs() < 0.01)
            });
            if !ok {
                v.push(ContractViolation {
                    check: "set_gain_confirm",
                    detail: format!("esperaba GainChanged{{ch:0,db:{mid_db}}}, obtuve {events:?}"),
                });
            }
        }
        Err(e) => v.push(ContractViolation {
            check: "set_gain_applied",
            detail: format!("comando válido rechazado: {e}"),
        }),
    }

    // --- Índice de canal inválido: error, nunca pánico ni éxito silencioso --
    match device.apply(DeviceCommand::SetGain {
        ch: profile.inputs,
        db: mid_db,
    }) {
        Err(_) => {}
        Ok(events) => v.push(ContractViolation {
            check: "invalid_channel_rejected",
            detail: format!("canal {} aceptado y devolvió {events:?}", profile.inputs),
        }),
    }

    // --- Phantom: ida y vuelta, solo si el modelo lo soporta --------------
    if profile.supports(Capabilities::PHANTOM_PER_CHANNEL) {
        match device.apply(DeviceCommand::SetPhantom { ch: 0, on: true }) {
            Ok(events) => {
                check_events(&mut v, &profile, "set_phantom", &events);
                let on = events
                    .iter()
                    .any(|e| matches!(e, DeviceEvent::PhantomChanged { ch: 0, on: true }));
                if !on {
                    v.push(ContractViolation {
                        check: "phantom_on_confirm",
                        detail: format!("esperaba PhantomChanged{{on:true}}, obtuve {events:?}"),
                    });
                }
            }
            Err(e) => v.push(ContractViolation {
                check: "phantom_on",
                detail: format!("phantom rechazado: {e}"),
            }),
        }
        let _ = device.apply(DeviceCommand::SetPhantom { ch: 0, on: false });
    }

    // --- Mute ------------------------------------------------------------
    if profile.supports(Capabilities::MUTE_PER_CHANNEL) {
        match device.apply(DeviceCommand::SetMute { ch: 0, on: true }) {
            Ok(events) => {
                check_events(&mut v, &profile, "set_mute", &events);
                let on = events
                    .iter()
                    .any(|e| matches!(e, DeviceEvent::MuteChanged { ch: 0, on: true }));
                if !on {
                    v.push(ContractViolation {
                        check: "mute_confirm",
                        detail: format!("esperaba MuteChanged{{on:true}}, obtuve {events:?}"),
                    });
                }
            }
            Err(e) => v.push(ContractViolation {
                check: "mute_applied",
                detail: format!("mute rechazado: {e}"),
            }),
        }
        let _ = device.apply(DeviceCommand::SetMute { ch: 0, on: false });
    }

    if let Err(e) = device.close() {
        v.push(ContractViolation {
            check: "close",
            detail: format!("close falló: {e}"),
        });
    }

    v
}

/// Ejecuta el contrato y **paniquea** con el detalle si algo falla.
///
/// Es la forma cómoda de usarlo dentro de un `#[test]`.
pub fn assert_device_contract(device: &mut dyn EvoDevice) {
    let violations = check_device_contract(device);
    if !violations.is_empty() {
        let joined = violations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n  - ");
        panic!("el dispositivo viola el contrato de EvoDevice:\n  - {joined}");
    }
}

/// Todo evento devuelto debe referenciar índices válidos de este perfil.
fn check_events(
    v: &mut Vec<ContractViolation>,
    profile: &evo_domain::DeviceProfile,
    check: &'static str,
    events: &[DeviceEvent],
) {
    for ev in events {
        let bad_channel = match ev {
            DeviceEvent::GainChanged { ch, .. }
            | DeviceEvent::MuteChanged { ch, .. }
            | DeviceEvent::PhantomChanged { ch, .. }
            | DeviceEvent::PadChanged { ch, .. }
            | DeviceEvent::InstrumentChanged { ch, .. } => !profile.has_input(*ch),
            DeviceEvent::MonitorMixChanged { out, .. } => !profile.has_output(*out),
            DeviceEvent::LoopbackChanged { .. }
            | DeviceEvent::SmartGainChanged { .. }
            | DeviceEvent::ConnectionChanged(_) => false,
        };
        if bad_channel {
            v.push(ContractViolation {
                check,
                detail: format!("evento con índice fuera del perfil: {ev:?}"),
            });
        }
    }
}
