//! Tests del núcleo puro.
//!
//! Todo lo que se prueba aquí lo hace **sin hardware, sin red y sin relojes**:
//! el dominio recibe `now` como parámetro, así que el *cooldown* de phantom se
//! verifica de forma determinista, sin `sleep` ni flakiness.

use std::time::Duration;

use evo_domain::{
    Capabilities, CoalesceKey, DeviceCommand, DeviceError, DeviceEvent, DeviceProfile, DeviceState,
    MeterFrame, ModelId, SafetyContext, SafetyError, SafetyLimits, SafetyPolicy, SmartGainPhase,
    StateOp, state_ops_from_events,
};

fn evo4() -> DeviceProfile {
    DeviceProfile::evo4()
}

fn evo16() -> DeviceProfile {
    DeviceProfile::evo16()
}

fn policy() -> SafetyPolicy {
    SafetyPolicy::default()
}

fn ctx() -> SafetyContext {
    SafetyContext::default()
}

// ---------------------------------------------------------------- ganancia --

#[test]
fn la_ganancia_se_ajusta_al_rango_del_perfil() {
    let profile = evo4();
    let state = DeviceState::initial(&profile);
    let too_high = profile.gain_range_db.max_db + 20.0;

    let cmd = policy()
        .validate(
            DeviceCommand::SetGain {
                ch: 0,
                db: too_high,
            },
            &profile,
            &state,
            &ctx(),
            Duration::ZERO,
        )
        .expect("un valor alto debe ajustarse, no rechazarse");

    match cmd {
        DeviceCommand::SetGain { db, .. } => assert_eq!(db, profile.gain_range_db.max_db),
        other => panic!("comando inesperado: {other:?}"),
    }
}

#[test]
fn un_valor_no_finito_se_rechaza_en_lugar_de_ajustarse() {
    let profile = evo4();
    let state = DeviceState::initial(&profile);

    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let err = policy()
            .validate(
                DeviceCommand::SetGain { ch: 0, db: bad },
                &profile,
                &state,
                &ctx(),
                Duration::ZERO,
            )
            .expect_err("NaN/Inf debe rechazarse");
        assert!(
            matches!(
                err,
                evo_domain::CommandRejection::Domain(evo_domain::DomainError::NotFinite { .. })
            ),
            "error inesperado para {bad}: {err:?}"
        );
    }
}

#[test]
fn un_canal_inexistente_se_rechaza() {
    let profile = evo4(); // 2 canales: índices 0 y 1
    let state = DeviceState::initial(&profile);

    let err = policy()
        .validate(
            DeviceCommand::SetGain { ch: 2, db: 10.0 },
            &profile,
            &state,
            &ctx(),
            Duration::ZERO,
        )
        .expect_err("el canal 2 no existe en el EVO 4");
    assert!(matches!(
        err,
        evo_domain::CommandRejection::Domain(evo_domain::DomainError::ChannelOutOfRange {
            ch: 2,
            channels: 2
        })
    ));
}

// ----------------------------------------------------------------- phantom --

#[test]
fn phantom_bloqueado_globalmente_se_rechaza() {
    let profile = evo4();
    let state = DeviceState::initial(&profile);
    let locked = SafetyContext {
        phantom_locked: true,
        ..SafetyContext::default()
    };

    let err = policy()
        .validate(
            DeviceCommand::SetPhantom { ch: 0, on: true },
            &profile,
            &state,
            &locked,
            Duration::ZERO,
        )
        .expect_err("phantom bloqueado");
    assert!(matches!(
        err,
        evo_domain::CommandRejection::Safety(SafetyError::PhantomLocked)
    ));
}

#[test]
fn phantom_respeta_la_ventana_de_enfriamiento() {
    let profile = evo4();
    let state = DeviceState::initial(&profile); // phantom apagado
    let limits = SafetyLimits::default();
    let policy = SafetyPolicy::new(limits);

    let context = SafetyContext {
        last_phantom_change_at: {
            let mut a = [None; evo_domain::MAX_CHANNELS];
            a[0] = Some(Duration::from_millis(10_000));
            a
        },
        ..SafetyContext::default()
    };

    // Solo 500 ms después del último cambio: todavía no.
    let err = policy
        .validate(
            DeviceCommand::SetPhantom { ch: 0, on: true },
            &profile,
            &state,
            &context,
            Duration::from_millis(10_500),
        )
        .expect_err("debe esperar al cooldown");
    match err {
        evo_domain::CommandRejection::Safety(SafetyError::PhantomCooldown { remaining_ms }) => {
            assert_eq!(remaining_ms, 1_500, "faltan 2 s - 500 ms transcurridos");
        }
        other => panic!("error inesperado: {other:?}"),
    }

    // Pasada la ventana, se permite.
    policy
        .validate(
            DeviceCommand::SetPhantom { ch: 0, on: true },
            &profile,
            &state,
            &context,
            Duration::from_millis(12_001),
        )
        .expect("pasado el cooldown debe permitirse");
}

#[test]
fn pedir_el_phantom_que_ya_esta_no_consume_cooldown() {
    let profile = evo4();
    let mut state = DeviceState::initial(&profile);
    state.channels[0].phantom = true; // ya está encendido

    let context = SafetyContext {
        phantom_locked: true, // incluso bloqueado…
        ..SafetyContext::default()
    };

    // …un no-op es seguro: no conmuta nada, así que no hay pop ni riesgo.
    policy()
        .validate(
            DeviceCommand::SetPhantom { ch: 0, on: true },
            &profile,
            &state,
            &context,
            Duration::ZERO,
        )
        .expect("el no-op no debe rechazarse por cooldown ni por el lock");
}

// -------------------------------------------------------------- capacidades --

#[test]
fn una_funcion_no_soportada_por_el_modelo_se_rechaza() {
    let profile = evo4(); // sin pad ni instrumento
    let state = DeviceState::initial(&profile);

    for cmd in [
        DeviceCommand::SetPad { ch: 0, on: true },
        DeviceCommand::SetInstrument { ch: 0, on: true },
    ] {
        let err = policy()
            .validate(cmd, &profile, &state, &ctx(), Duration::ZERO)
            .expect_err("el EVO 4 no soporta esta función");
        assert!(matches!(
            err,
            evo_domain::CommandRejection::Safety(SafetyError::Unsupported { .. })
        ));
    }
}

#[test]
fn el_evo4_soporta_el_mezclador_de_monitor_y_volumen() {
    let profile = evo4();
    let state = DeviceState::initial(&profile);

    policy()
        .validate(
            DeviceCommand::SetMonitorMix {
                out: 0,
                level_db: -12.0,
                pan: 0.2,
            },
            &profile,
            &state,
            &ctx(),
            Duration::ZERO,
        )
        .expect("el EVO 4 declara MONITOR_MIX");
}

#[test]
fn el_evo16_si_soporta_el_mezclador_de_monitor() {
    let profile = evo16();
    let state = DeviceState::initial(&profile);

    policy()
        .validate(
            DeviceCommand::SetMonitorMix {
                out: 7,
                level_db: -3.0,
                pan: -0.5,
            },
            &profile,
            &state,
            &ctx(),
            Duration::ZERO,
        )
        .expect("el EVO 16 declara MONITOR_MIX");
}

#[test]
fn smartgain_no_admite_dos_ejecuciones_concurrentes() {
    let profile = evo4();
    let state = DeviceState::initial(&profile);
    let running = SafetyContext {
        smartgain_running: true,
        ..SafetyContext::default()
    };

    let err = policy()
        .validate(
            DeviceCommand::TriggerSmartGain,
            &profile,
            &state,
            &running,
            Duration::ZERO,
        )
        .expect_err("no puede haber dos SmartGain a la vez");
    assert!(matches!(
        err,
        evo_domain::CommandRejection::Safety(SafetyError::SmartGainBusy)
    ));
}

#[test]
fn cancelar_siempre_esta_permitido() {
    let profile = evo4();
    let state = DeviceState::initial(&profile);
    let hostile = SafetyContext {
        phantom_locked: true,
        smartgain_running: true,
        ..SafetyContext::default()
    };

    policy()
        .validate(
            DeviceCommand::CancelSmartGain,
            &profile,
            &state,
            &hostile,
            Duration::ZERO,
        )
        .expect("cancelar siempre es seguro");
}

// ------------------------------------------------------------------- estado --

#[test]
fn aplicar_un_lote_es_atomico() {
    let profile = evo4();
    let mut state = DeviceState::initial(&profile);

    let ops = vec![
        StateOp::Gain { ch: 0, db: 12.0 },
        StateOp::Mute { ch: 1, on: true },
        // Este op es inválido y debe abortar el lote completo.
        StateOp::Gain { ch: 9, db: 0.0 },
    ];

    state
        .apply_all(&ops)
        .expect_err("el lote contiene un canal inválido");

    assert_eq!(
        state.channels[0].gain_db, 0.0,
        "el primer op no debe haberse aplicado"
    );
    assert!(!state.channels[1].muted, "el segundo op tampoco");
}

#[test]
fn los_eventos_se_traducen_a_deltas_con_ruta_estable() {
    let events = [
        DeviceEvent::GainChanged { ch: 0, db: 31.5 },
        DeviceEvent::PhantomChanged { ch: 1, on: true },
        DeviceEvent::LoopbackChanged { on: true },
        // Efímero: no forma parte del estado canónico.
        DeviceEvent::ConnectionChanged(evo_domain::ConnectionState::Degraded),
    ];

    let ops = state_ops_from_events(&events);
    assert_eq!(ops.len(), 3);
    assert_eq!(ops[0].path(), "ch/0/gain");
    assert_eq!(ops[1].path(), "ch/1/phantom");
    assert_eq!(ops[2].path(), "loopback");
}

#[test]
fn dos_ops_del_mismo_parametro_comparten_clave() {
    let a = StateOp::MonitorMix {
        out: 1,
        level_db: -6.0,
        pan: 0.0,
    };
    let b = StateOp::MonitorMix {
        out: 1,
        level_db: 3.0,
        pan: 0.9,
    };
    assert_eq!(
        a.param_key(),
        b.param_key(),
        "last-write-wins por parámetro"
    );
}

#[test]
fn cancelar_smartgain_es_terminal_y_dispararlo_no_lo_es() {
    assert!(SmartGainPhase::Cancelled.is_terminal());
    assert!(SmartGainPhase::Success.is_terminal());
    assert!(SmartGainPhase::Idle.is_terminal());
    assert!(!SmartGainPhase::Listening.is_terminal());
    assert!(SmartGainPhase::Listening.is_running());
    assert!(!SmartGainPhase::Idle.is_running());
}

// ---------------------------------------------------------------- coalescing --

#[test]
fn los_efectos_de_borde_no_se_coalescen() {
    assert_eq!(
        DeviceCommand::SetGain { ch: 0, db: 1.0 }.coalesce_key(),
        Some(CoalesceKey::Gain(0))
    );
    assert_eq!(
        DeviceCommand::SetGain { ch: 1, db: 1.0 }.coalesce_key(),
        Some(CoalesceKey::Gain(1))
    );
    assert_eq!(DeviceCommand::TriggerSmartGain.coalesce_key(), None);
    assert_eq!(DeviceCommand::CancelSmartGain.coalesce_key(), None);
}

#[test]
fn el_nombre_serializado_de_cada_comando_coincide_con_op_name() {
    // El contrato IPC depende de que el `op` del cable sea estable; que coincida
    // con `op_name()` (usado en logs/auditoría) evita dos nombres para lo mismo.
    let commands = [
        DeviceCommand::SetGain { ch: 0, db: 0.0 },
        DeviceCommand::SetMute { ch: 0, on: true },
        DeviceCommand::SetPhantom { ch: 0, on: true },
        DeviceCommand::SetMonitorMix {
            out: 0,
            level_db: 0.0,
            pan: 0.0,
        },
        DeviceCommand::SetLoopback { on: true },
        DeviceCommand::TriggerSmartGain,
        DeviceCommand::CancelSmartGain,
        DeviceCommand::SetPad { ch: 0, on: true },
        DeviceCommand::SetInstrument { ch: 0, on: true },
    ];
    for cmd in commands {
        let value = serde_json::to_value(cmd).expect("serializa");
        assert_eq!(
            value["op"].as_str(),
            Some(cmd.op_name()),
            "op_name de {cmd:?} no coincide con el nombre serializado"
        );
    }
}

// -------------------------------------------------------------------- meter --

#[test]
fn un_fotograma_de_medidores_se_codifica_como_el_contrato_binario() {
    let frame = MeterFrame::from_db_fs(0x1234, &[-3.0, -60.0, 0.0]);
    let bytes = frame.encode();

    // [0x01][seq:u16 LE][n:u8][peak_i8 × n]
    assert_eq!(bytes[0], evo_domain::meter::METER_FRAME_TAG);
    assert_eq!(&bytes[1..3], &0x1234u16.to_le_bytes());
    assert_eq!(bytes[3], 3);
    assert_eq!(bytes.len(), 4 + 3);
    assert_eq!(bytes[4] as i8, -3);
    assert_eq!(bytes[5] as i8, -60);
    assert_eq!(bytes[6] as i8, 0);
}

#[test]
fn un_fotograma_codificado_se_decodifica_sin_perdida() {
    let frame = MeterFrame::from_db_fs(0x1234, &[-3.0, -60.0, 0.0]);
    let decoded = MeterFrame::decode(&frame.encode()).expect("fotograma válido");

    assert_eq!(decoded, frame);
    assert_eq!(decoded.seq, 0x1234);
    assert_eq!(decoded.channels(), 3);
    assert_eq!(decoded.peaks(), frame.peaks());
}

#[test]
fn se_rechazan_fotogramas_con_etiqueta_o_longitud_invalidas() {
    let frame = MeterFrame::silent(1, 4);
    let bytes = frame.encode();

    // Etiqueta equivocada.
    let mut bad_tag = bytes.clone();
    bad_tag[0] = 0x02;
    assert_eq!(MeterFrame::decode(&bad_tag), None);

    // Truncado (faltan picos).
    assert_eq!(MeterFrame::decode(&bytes[..bytes.len() - 1]), None);

    // `n` por encima de la capacidad de dimensionamiento.
    let mut too_many = bytes.clone();
    too_many[3] = (evo_domain::MAX_CHANNELS + 1) as u8;
    assert_eq!(MeterFrame::decode(&too_many), None);

    assert_eq!(MeterFrame::decode(&[]), None);
}

#[test]
fn los_valores_no_finitos_se_cuantizan_como_silencio() {
    let frame = MeterFrame::from_db_fs(0, &[f32::NAN, f32::INFINITY, -200.0]);

    // Silencio = -128, y el fondo también se recorta ahí.
    assert_eq!(frame.peak_db_fs(0), Some(-128.0));
    assert_eq!(frame.peak_db_fs(1), Some(-128.0));
    assert_eq!(frame.peak_db_fs(2), Some(-128.0));
}

#[test]
fn no_sobran_canales_fuera_de_la_capacidad() {
    let profile = evo16();
    assert!(profile.inputs as usize <= evo_domain::MAX_CHANNELS);
    assert!(profile.outputs as usize <= evo_domain::MAX_OUTPUTS);
}

// ----------------------------------------------------------- modelo/perfil --

#[test]
fn los_modelos_se_resuelven_por_codigo_estable() {
    assert_eq!(ModelId::Evo4.code(), "evo4");
    assert_eq!(ModelId::Evo16.code(), "evo16");
    assert_eq!(ModelId::VENDOR_ID, 0x2708);
    assert_eq!(ModelId::ALL.len(), 3);
}

#[test]
fn los_pid_estan_deliberadamente_sin_definir() {
    // Si esto falla, alguien inventó un Product ID: hay que confirmarlo con
    // `lsusb` sobre hardware real antes de fijarlo (SAD §8.7).
    for model in ModelId::ALL {
        assert_eq!(model.product_id(), None);
    }
}

#[test]
fn el_perfil_del_evo4_declara_lo_que_el_producto_necesita() {
    let profile = evo4();
    assert_eq!(profile.inputs, 2);
    assert_eq!(profile.outputs, 2);
    assert!(profile.supports(Capabilities::PHANTOM_PER_CHANNEL));
    assert!(profile.supports(Capabilities::SMARTGAIN));
    assert!(profile.supports(Capabilities::LOOPBACK));
    assert!(profile.supports(Capabilities::MONITOR_MIX));
    assert!(!profile.supports(Capabilities::INPUT_PAD));
}

#[test]
fn los_errores_de_dispositivo_son_comparables() {
    assert_eq!(DeviceError::Timeout, DeviceError::Timeout);
    assert_ne!(
        DeviceError::NotFound,
        DeviceError::Unsupported("read_state")
    );
}
