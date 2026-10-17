//! El contrato de `EvoDevice` debe cumplirse para **todos** los modelos.
//!
//! Esta es la promesa del SAD §2.5 hecha ejecutable: la misma suite que corre
//! contra el mock correrá contra el driver USB real, sin escribir tests nuevos.

use evo_domain::{DeviceCommand, DeviceError, DeviceProfile, ModelId, SmartGainPhase};
use evo_mock::{Fault, InMemoryStateRepo, MockDevice, MockDriverFactory, MockMeterSource};
use evo_ports::{DriverRegistry, EvoDevice, MeterSource, StateRepo};

use evo_domain::{DeviceState, PersistedState};

#[test]
fn el_mock_cumple_el_contrato_en_todos_los_modelos() {
    for model in ModelId::ALL {
        let mut device = MockDevice::new(model);
        let violations = evo_ports::testkit::check_device_contract(&mut device);
        assert!(
            violations.is_empty(),
            "el mock del {model} viola el contrato:\n{violations:#?}"
        );
    }
}

#[test]
fn read_state_no_esta_soportado_por_defecto() {
    // Es el comportamiento honesto de v1.0: el hardware no se puede leer
    // (SAD §8.6). Que sea `Unsupported` y no un valor inventado es el punto.
    let mut device = MockDevice::new(ModelId::Evo4);
    assert_eq!(
        device.read_state(),
        Err(DeviceError::Unsupported("read_state"))
    );
}

#[test]
fn read_state_funciona_cuando_el_hardware_lo_permite() {
    let mut device = MockDevice::new(ModelId::Evo4).with_read_state();
    let state = device.read_state().expect("habilitado explícitamente");
    assert_eq!(state.channels.len(), 2);
}

#[test]
fn una_desconexion_a_mitad_de_la_rafaga_es_recuperable() {
    let mut device = MockDevice::new(ModelId::Evo4);
    device
        .apply(DeviceCommand::SetGain { ch: 0, db: 20.0 })
        .expect("primer comando ok");

    device.schedule_fault(Fault::Disconnect);
    let err = device
        .apply(DeviceCommand::SetGain { ch: 0, db: 24.0 })
        .expect_err("el dispositivo se fue");
    assert_eq!(err, DeviceError::Disconnected);

    // Todo comando posterior falla mientras siga caído…
    assert_eq!(
        device.apply(DeviceCommand::SetMute { ch: 0, on: true }),
        Err(DeviceError::Disconnected)
    );

    // …y se recupera al reconectar.
    assert!(device.reconnect());
    device
        .apply(DeviceCommand::SetMute { ch: 0, on: true })
        .expect("tras reconectar vuelve a aceptar comandos");
}

#[test]
fn un_fallo_de_transferencia_no_mata_el_dispositivo() {
    let mut device = MockDevice::new(ModelId::Evo4);
    device.schedule_fault(Fault::TransferError("pipe stalled"));

    let err = device
        .apply(DeviceCommand::SetGain { ch: 0, db: 10.0 })
        .expect_err("la transferencia falló");
    assert!(matches!(err, DeviceError::Transfer(_)));

    device
        .apply(DeviceCommand::SetGain { ch: 0, db: 10.0 })
        .expect("el siguiente intento sí funciona");
}

#[test]
fn el_mock_rechaza_lo_que_el_modelo_no_soporta() {
    let mut device = MockDevice::new(ModelId::Evo4);
    // El EVO 4 no declara pad ni entrada de instrumento.
    assert_eq!(
        device.apply(DeviceCommand::SetPad { ch: 0, on: true }),
        Err(DeviceError::Unsupported("pad de entrada"))
    );
    assert!(matches!(
        device.apply(DeviceCommand::SetInstrument { ch: 0, on: true }),
        Err(DeviceError::Unsupported(_))
    ));
    // Y sí soporta mezclador de monitor
    assert!(
        device
            .apply(DeviceCommand::SetMonitorMix {
                out: 0,
                level_db: -10.0,
                pan: 0.0,
            })
            .is_ok()
    );
}

#[test]
fn el_mock_registra_los_comandos_para_poder_medir_coalescing() {
    let mut device = MockDevice::new(ModelId::Evo4);
    for i in 0..200u32 {
        device
            .apply(DeviceCommand::SetGain {
                ch: 0,
                db: (i % 57) as f32,
            })
            .expect("en rango");
    }
    // El adaptador aplicó las 200: es el actor (evo-core) quien debe coalescer
    // antes de llegar aquí (SAD §2.2). El mock solo deja evidencia.
    assert_eq!(device.applied_commands().len(), 200);
    device.clear_applied();
    assert!(device.applied_commands().is_empty());
}

#[test]
fn smartgain_avanza_en_el_mock() {
    let mut device = MockDevice::new(ModelId::Evo4);
    let events = device
        .apply(DeviceCommand::TriggerSmartGain)
        .expect("el EVO 4 soporta SmartGain");
    assert!(events.iter().any(|e| matches!(
        e,
        evo_domain::DeviceEvent::SmartGainChanged {
            phase: SmartGainPhase::Listening
        }
    )));
}

// ---------------------------------------------------------------- registro --

#[test]
fn el_registro_resuelve_por_modelo() {
    let mut registry = DriverRegistry::new();
    for model in ModelId::ALL {
        registry.register(Box::new(MockDriverFactory::new(model)));
    }

    assert_eq!(registry.len(), 3);
    let factory = registry.by_model(ModelId::Evo16).expect("registrado");
    assert_eq!(factory.profile().inputs, 8);

    let device = factory.open().expect("abre un mock");
    assert_eq!(device.profile().model, ModelId::Evo16);
}

#[test]
fn el_registro_sin_pid_confirmado_no_resuelve_por_ids() {
    let mut registry = DriverRegistry::new();
    registry.register(Box::new(MockDriverFactory::new(ModelId::Evo4)));

    // Sin PID confirmado (SAD §8.7), buscar por identidad USB no encuentra nada:
    // es preferible fallar a reclamar un dispositivo equivocado.
    assert!(registry.known_device_ids().is_empty());
    assert!(
        registry
            .by_ids(evo_ports::DeviceId::new(ModelId::VENDOR_ID, 0x0001))
            .is_none()
    );
}

#[test]
fn el_registro_resuelve_por_ids_cuando_se_conocen() {
    let mut registry = DriverRegistry::new();
    registry.register(Box::new(MockDriverFactory::with_device_id(
        ModelId::Evo4,
        0x0001,
    )));

    let found = registry
        .by_ids(evo_ports::DeviceId::new(ModelId::VENDOR_ID, 0x0001))
        .expect("identidad registrada");
    assert_eq!(found.model(), ModelId::Evo4);
}

// ------------------------------------------------------------ medidores -

#[test]
fn los_medidores_son_deterministas_y_planos() {
    let profile = DeviceProfile::evo4();
    let mut source = MockMeterSource::new();
    source.start(&profile).expect("arranca");

    let a = source.latest();
    let b = source.latest();
    assert_eq!(a, b, "latest() no debe tener efectos secundarios");

    assert_eq!(a.channels(), 2);
    source.advance();
    let c = source.latest();
    assert_ne!(a, c, "al avanzar la señal, la lectura cambia");
    assert_eq!(c.seq, 1);
}

// ------------------------------------------------------------ persistencia -

#[test]
fn el_repo_en_memoria_guarda_y_devuelve() {
    let repo = InMemoryStateRepo::new();
    assert_eq!(repo.load().expect("load"), None);

    let profile = DeviceProfile::evo4();
    let doc = PersistedState::new(
        profile.clone(),
        DeviceState::initial(&profile),
        1_700_000_000,
    );

    repo.save(&doc).expect("save");
    assert_eq!(repo.load().expect("load"), Some(doc));
}
