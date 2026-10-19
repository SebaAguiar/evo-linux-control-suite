//! Tests de integración del núcleo: `evo-core` contra `evo-mock`.
//!
//! Todo lo que aquí se ejercita pasa por el camino real (canal → actor → store →
//! difusión); no se llama a ninguna función interna directamente. Si el modelo de
//! concurrencia del SAD §2.1 estuviera mal, estos tests lo notarían.

use std::sync::Arc;
use std::time::Duration;

use evo_core::{Coalescer, CoreConfig, CoreError, EvoCore, ReconnectPolicy, StateStore};
use evo_domain::ModelId;
use evo_domain::{
    CommandRejection, ConnectionState, DeviceCommand, DeviceEvent, DeviceProfile, DeviceState,
    SafetyError, SmartGainPhase, StateSnapshot,
};
use evo_mock::{Fault, MockDevice};
use tokio::sync::watch;

/// Espera a que la instantánea cumpla una condición, con tope de tiempo.
///
/// Es el patrón que usará un cliente de verdad: mirar el estado, y si no es el
/// esperado, esperar el siguiente cambio. El tope evita que un fallo del núcleo
/// cuelgue la suite para siempre.
async fn wait_for(
    rx: &mut watch::Receiver<StateSnapshot>,
    what: &str,
    pred: impl Fn(&StateSnapshot) -> bool,
) -> StateSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = rx.borrow_and_update().clone();
            if pred(&current) {
                return current;
            }
            if rx.changed().await.is_err() {
                panic!("el núcleo se apagó mientras esperaba {what}");
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timeout esperando {what}"))
}

// ------------------------------------------------------------- coalescer --

#[test]
fn el_coalescer_conserva_solo_el_ultimo_valor_por_parametro() {
    let mut c = Coalescer::new(Duration::from_millis(12));
    let t = Duration::ZERO;

    c.push(DeviceCommand::SetGain { ch: 0, db: 1.0 }, t);
    c.push(DeviceCommand::SetGain { ch: 0, db: 2.0 }, t);
    c.push(DeviceCommand::SetGain { ch: 0, db: 3.0 }, t);

    assert_eq!(c.flush(), vec![DeviceCommand::SetGain { ch: 0, db: 3.0 }]);
}

#[test]
fn el_coalescer_trata_cada_canal_por_separado() {
    let mut c = Coalescer::new(Duration::from_millis(12));
    let t = Duration::ZERO;

    c.push(DeviceCommand::SetGain { ch: 0, db: 1.0 }, t);
    c.push(DeviceCommand::SetGain { ch: 1, db: 5.0 }, t);
    c.push(DeviceCommand::SetGain { ch: 0, db: 2.0 }, t);

    assert_eq!(
        c.flush(),
        vec![
            DeviceCommand::SetGain { ch: 1, db: 5.0 },
            DeviceCommand::SetGain { ch: 0, db: 2.0 },
        ]
    );
}

#[test]
fn el_coalescer_no_descarta_comandos_de_borde() {
    let mut c = Coalescer::new(Duration::from_millis(12));
    let t = Duration::ZERO;

    c.push(DeviceCommand::SetPhantom { ch: 0, on: true }, t);
    c.push(DeviceCommand::SetPhantom { ch: 0, on: false }, t);

    // El phantom tiene clave de coalescing (es un parámetro con estado)…
    assert_eq!(
        c.flush(),
        vec![DeviceCommand::SetPhantom { ch: 0, on: false }]
    );

    // …pero SmartGain no: cada disparo cuenta.
    c.push(DeviceCommand::TriggerSmartGain, t);
    c.push(DeviceCommand::CancelSmartGain, t);
    assert_eq!(
        c.flush(),
        vec![
            DeviceCommand::TriggerSmartGain,
            DeviceCommand::CancelSmartGain
        ]
    );
}

#[test]
fn la_ventana_vencida_se_detecta_por_marca_de_tiempo() {
    let mut c = Coalescer::new(Duration::from_millis(10));
    c.push(DeviceCommand::TriggerSmartGain, Duration::from_millis(100));

    assert!(!c.has_due(Duration::from_millis(105)));
    assert_eq!(
        c.time_until_due(Duration::from_millis(105)),
        Some(Duration::from_millis(5))
    );
    assert!(c.has_due(Duration::from_millis(110)));
    assert_eq!(
        c.time_until_due(Duration::from_millis(110)),
        Some(Duration::ZERO)
    );
}

// ----------------------------------------------------------------- store --

#[test]
fn el_store_solo_avanza_la_revision_cuando_algo_cambia() {
    let profile = DeviceProfile::evo4();
    let mut store = StateStore::initial(profile);

    let ev = store
        .apply(&[evo_domain::StateOp::Gain { ch: 0, db: 12.0 }])
        .expect("delta válido");
    assert_eq!(ev.map(|e| e.rev), Some(1));

    // Mismo valor otra vez: no hay delta, no hay revisión nueva.
    let ev = store
        .apply(&[evo_domain::StateOp::Gain { ch: 0, db: 12.0 }])
        .expect("delta válido");
    assert_eq!(ev, None);
    assert_eq!(store.rev(), 1);

    // Un lote con un índice inválido no se aplica a medias.
    let err = store.apply(&[
        evo_domain::StateOp::Gain { ch: 1, db: 4.0 },
        evo_domain::StateOp::Gain { ch: 9, db: 4.0 },
    ]);
    assert!(err.is_err());
    assert_eq!(store.rev(), 1, "un lote fallido no consume revisión");
    assert_eq!(store.state().channels[1].gain_db, 0.0);
}

// ---------------------------------------------------- núcleo en ejecución --

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_comando_valido_se_aplica_y_avanza_la_revision() {
    let core = EvoCore::spawn(
        Box::new(MockDevice::new(ModelId::Evo4)),
        CoreConfig::default(),
    );
    let mut snapshots = core.subscribe_snapshots();

    assert_eq!(core.snapshot().rev, 0);

    core.submit(DeviceCommand::SetGain { ch: 0, db: 24.0 })
        .await
        .expect("comando válido");

    let snap = wait_for(&mut snapshots, "la ganancia aplicada", |s| {
        s.state.channels[0].gain_db == 24.0
    })
    .await;
    assert_eq!(snap.rev, 1);
    assert_eq!(snap.profile.model, ModelId::Evo4);

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_comando_idempotente_no_genera_delta() {
    let core = EvoCore::spawn(
        Box::new(MockDevice::new(ModelId::Evo4)),
        CoreConfig::default(),
    );
    let mut snapshots = core.subscribe_snapshots();

    for _ in 0..3 {
        core.submit(DeviceCommand::SetMute { ch: 0, on: true })
            .await
            .expect("aceptado");
    }

    wait_for(&mut snapshots, "el mute aplicado", |s| {
        s.state.channels[0].muted
    })
    .await;
    // Margen para que, si hubiese deltas de más, se hubieran notado.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        core.snapshot().rev,
        1,
        "solo una revisión para un cambio real"
    );

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn la_ganancia_fuera_de_rango_se_ajusta_y_el_indice_invalido_se_rechaza() {
    let core = EvoCore::spawn(
        Box::new(MockDevice::new(ModelId::Evo4)),
        CoreConfig::default(),
    );
    let mut snapshots = core.subscribe_snapshots();

    // Dentro de un índice válido pero fuera de rango: se ajusta al máximo.
    core.submit(DeviceCommand::SetGain { ch: 0, db: 9_999.0 })
        .await
        .expect("se ajusta, no se rechaza");
    let max = core.snapshot().profile.gain_range_db.max_db;
    let snap = wait_for(&mut snapshots, "el recorte de ganancia", |s| {
        s.state.channels[0].gain_db == max
    })
    .await;
    assert_eq!(snap.state.channels[0].gain_db, max);

    // Índice fuera del perfil: se rechaza antes de tocar el hardware.
    let err = core
        .submit(DeviceCommand::SetGain { ch: 7, db: 0.0 })
        .await
        .expect_err("el EVO 4 tiene 2 canales");
    assert!(matches!(
        err,
        CoreError::Rejected(CommandRejection::Domain(_))
    ));

    // NaN: se rechaza, nunca se "ajusta" a un valor sin sentido.
    let err = core
        .submit(DeviceCommand::SetGain {
            ch: 0,
            db: f32::NAN,
        })
        .await
        .expect_err("NaN no es un valor de ganancia");
    assert!(matches!(
        err,
        CoreError::Rejected(CommandRejection::Domain(_))
    ));

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn el_phantom_respeta_el_enfriamiento_y_el_bloqueo_global() {
    let core = EvoCore::spawn(
        Box::new(MockDevice::new(ModelId::Evo4)),
        CoreConfig::default(),
    );
    let mut snapshots = core.subscribe_snapshots();

    core.submit(DeviceCommand::SetPhantom { ch: 0, on: true })
        .await
        .expect("encender phantom es seguro la primera vez");
    wait_for(&mut snapshots, "phantom encendido", |s| {
        s.state.channels[0].phantom
    })
    .await;

    // Apagarlo de inmediato viola el cooldown de 2 s.
    let err = core
        .submit(DeviceCommand::SetPhantom { ch: 0, on: false })
        .await
        .expect_err("demasiado pronto");
    assert!(matches!(
        err,
        CoreError::Rejected(CommandRejection::Safety(
            SafetyError::PhantomCooldown { .. }
        ))
    ));

    // El bloqueo global rechaza incluso el primer encendido del canal 1.
    core.set_phantom_lock(true).await.expect("bloqueo");
    let err = core
        .submit(DeviceCommand::SetPhantom { ch: 1, on: true })
        .await
        .expect_err("bloqueado");
    assert!(matches!(
        err,
        CoreError::Rejected(CommandRejection::Safety(SafetyError::PhantomLocked))
    ));

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn una_rafaga_de_slider_se_coalesce_en_el_bus() {
    // La latencia simulada es la que hace que el actor esté "ocupado" mientras
    // llega el resto de la ráfaga, que es cuando el coalescing tiene sentido.
    let device = MockDevice::new(ModelId::Evo4).with_apply_delay(Duration::from_millis(1));
    let log = device.log();

    let core = Arc::new(EvoCore::spawn(
        Box::new(device),
        CoreConfig {
            coalesce_window: Duration::from_millis(12),
            ..CoreConfig::default()
        },
    ));

    const BURST: u32 = 200;
    let mut tasks = Vec::new();
    for i in 0..BURST {
        let core = Arc::clone(&core);
        tasks.push(tokio::spawn(async move {
            core.submit(DeviceCommand::SetGain {
                ch: 0,
                db: (i % 57) as f32,
            })
            .await
        }));
    }
    for task in tasks {
        task.await.expect("tarea").expect("comando aceptado");
    }

    // Los envíos concurrentes no tienen un orden determinista, así que no se
    // puede afirmar *qué* valor quedó, sí que el bus se vació y que lo que quedó
    // es exactamente el último comando que llegó al hardware.
    let applied = settle(&log).await;
    assert!(
        applied < (BURST as usize) / 4,
        "el coalescing debería colapsar la ráfaga, pero llegaron {applied} de {BURST} comandos"
    );

    let last_db = match log.lock().expect("mutex").last() {
        Some(DeviceCommand::SetGain { db, .. }) => *db,
        other => panic!("el último comando aplicado no fue SetGain: {other:?}"),
    };
    assert_eq!(
        core.snapshot().state.channels[0].gain_db,
        last_db,
        "el estado final debe ser el último valor aplicado (last-write-wins)"
    );

    let core = Arc::try_unwrap(core).expect("sin clones vivos");
    core.shutdown().expect("apagado limpio");
}

/// Espera a que el registro de comandos deje de crecer.
///
/// Es la forma robusta de observar "el bus ya se vació" sin depender de un
/// `sleep` con una duración adivinada.
async fn settle(log: &Arc<std::sync::Mutex<Vec<DeviceCommand>>>) -> usize {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut last = log.lock().expect("mutex").len();
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let now = log.lock().expect("mutex").len();
            if now == last {
                return now;
            }
            last = now;
        }
    })
    .await
    .expect("el bus no se estabilizó")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_comando_rechazado_nunca_llega_al_hardware() {
    let device = MockDevice::new(ModelId::Evo4);
    let log = device.log();
    let core = EvoCore::spawn(Box::new(device), CoreConfig::default());

    let err = core
        .submit(DeviceCommand::SetPad { ch: 0, on: true })
        .await
        .expect_err("el EVO 4 no tiene pad de entrada");
    assert!(matches!(
        err,
        CoreError::Rejected(CommandRejection::Safety(SafetyError::Unsupported { .. }))
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        log.lock().expect("mutex").is_empty(),
        "un comando rechazado no debe haberse aplicado"
    );

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smartgain_queda_ocupado_hasta_que_termina() {
    let core = EvoCore::spawn(
        Box::new(MockDevice::new(ModelId::Evo4)),
        CoreConfig::default(),
    );
    let mut snapshots = core.subscribe_snapshots();

    core.submit(DeviceCommand::TriggerSmartGain)
        .await
        .expect("primer disparo");
    wait_for(&mut snapshots, "SmartGain escuchando", |s| {
        s.state.smartgain == SmartGainPhase::Listening
    })
    .await;

    // Un segundo disparo mientras corre se rechaza (SAD §8.4).
    let err = core
        .submit(DeviceCommand::TriggerSmartGain)
        .await
        .expect_err("ya hay uno en curso");
    assert!(matches!(
        err,
        CoreError::Rejected(CommandRejection::Safety(SafetyError::SmartGainBusy))
    ));

    // Cancelar siempre es seguro y libera el recurso.
    core.submit(DeviceCommand::CancelSmartGain)
        .await
        .expect("cancelar siempre se acepta");
    wait_for(&mut snapshots, "SmartGain cancelado", |s| {
        s.state.smartgain == SmartGainPhase::Cancelled
    })
    .await;

    core.submit(DeviceCommand::TriggerSmartGain)
        .await
        .expect("tras cancelar se puede volver a disparar");

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn al_desconectar_se_reconecta_y_se_reaplica_el_estado_comandado() {
    let first = MockDevice::new(ModelId::Evo4);
    let faults = first.fault_handle();

    // Cada reapertura crea un mock nuevo; se guarda su registro para comprobar
    // que la reaplicación escribió de verdad en el dispositivo nuevo.
    let second_log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log_for_factory = Arc::clone(&second_log);
    let reopen: evo_ports::ReopenFn = Box::new(move || {
        Ok(Box::new(
            MockDevice::new(ModelId::Evo4).with_log(Arc::clone(&log_for_factory)),
        ))
    });

    let config = CoreConfig {
        reconnect: ReconnectPolicy {
            enabled: true,
            delay: Duration::from_millis(10),
            replay: true,
        },
        ..CoreConfig::default()
    };

    let core = EvoCore::spawn_with_reopen(Box::new(first), Some(reopen), config);
    let mut snapshots = core.subscribe_snapshots();
    let mut connection = core.subscribe_connection();

    // 1) Estado comandado antes del corte: es lo que hay que reaplicar.
    core.submit(DeviceCommand::SetGain { ch: 0, db: 30.0 })
        .await
        .expect("aceptado");
    wait_for(&mut snapshots, "ganancia inicial", |s| {
        s.state.channels[0].gain_db == 30.0
    })
    .await;

    // 2) Se suelta el cable y se manda un comando que lo descubre.
    *faults.lock().expect("mutex") = Some(Fault::Disconnect);
    core.submit(DeviceCommand::SetMute { ch: 0, on: true })
        .await
        .expect("aceptado y encolado");

    // 3) Debe verse el ciclo Disconnected → (reconexión) → Connected.
    wait_for(&mut snapshots, "el mute reaplicado", |s| {
        s.state.channels[0].muted
    })
    .await;
    assert_eq!(
        core.connection(),
        ConnectionState::Connected,
        "tras reconectar debe quedar conectado"
    );

    // 4) El dispositivo nuevo recibió la ganancia comandada (replay) y el mute
    //    pendiente. El phantom, en cambio, no se reaplica nunca por su cuenta.
    let applied = second_log.lock().expect("mutex").clone();
    assert!(
        applied.contains(&DeviceCommand::SetGain { ch: 0, db: 30.0 }),
        "el estado comandado debe reaplicarse al reconectar: {applied:?}"
    );
    assert!(
        applied.contains(&DeviceCommand::SetMute { ch: 0, on: true }),
        "el comando pendiente debe aplicarse tras reconectar: {applied:?}"
    );
    assert!(
        !applied
            .iter()
            .any(|c| matches!(c, DeviceCommand::SetPhantom { .. })),
        "el phantom no se reaplica automáticamente (SAD §8.5)"
    );

    // El canal de conexión es efímero y refleja el último valor.
    let _ = connection.borrow_and_update();

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn la_reconexion_insiste_hasta_que_el_cable_vuelve() {
    // El dispositivo no reaparece hasta el tercer intento: modela exactamente
    // "el usuario tarda en volver a enchufar", que es el caso real de §8.6.
    let first = MockDevice::new(ModelId::Evo4);
    let faults = first.fault_handle();

    let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let attempts_in_factory = Arc::clone(&attempts);
    let reopen: evo_ports::ReopenFn = Box::new(move || {
        let n = attempts_in_factory.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < 2 {
            Err(evo_domain::DeviceError::NotFound)
        } else {
            Ok(Box::new(MockDevice::new(ModelId::Evo4)))
        }
    });

    let config = CoreConfig {
        reconnect: ReconnectPolicy {
            enabled: true,
            delay: Duration::from_millis(10),
            replay: false,
        },
        ..CoreConfig::default()
    };

    let core = EvoCore::spawn_with_reopen(Box::new(first), Some(reopen), config);
    let mut snapshots = core.subscribe_snapshots();

    // Los cambios de conexión son efímeros: `watch` solo conserva el último
    // valor, así que hay que registrarlos en vivo para poder afirmar que hubo un
    // "buscando".
    let seen_connection = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut connection = core.subscribe_connection();
    let seen = Arc::clone(&seen_connection);
    let watcher = tokio::spawn(async move {
        seen.lock()
            .expect("mutex")
            .push(*connection.borrow_and_update());
        while connection.changed().await.is_ok() {
            seen.lock().expect("mutex").push(*connection.borrow());
        }
    });

    // Comando que descubre que el cable se soltó.
    *faults.lock().expect("mutex") = Some(Fault::Disconnect);
    core.submit(DeviceCommand::SetGain { ch: 0, db: 12.0 })
        .await
        .expect("aceptado");

    // Tras reconectar (al tercer intento), el comando pendiente se aplica.
    wait_for(
        &mut snapshots,
        "el comando pendiente aplicado tras reconectar",
        |s| s.state.channels[0].gain_db == 12.0,
    )
    .await;
    assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    assert_eq!(core.connection(), ConnectionState::Connected);

    core.shutdown().expect("apagado limpio");
    watcher.await.expect("observador");

    let states = seen_connection.lock().expect("mutex").clone();
    assert!(
        states.contains(&ConnectionState::Enumerating),
        "mientras reintenta, la UI debe poder mostrar que está buscando: {states:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sin_politica_de_reconexion_la_desconexion_es_definitiva() {
    let first = MockDevice::new(ModelId::Evo4);
    let faults = first.fault_handle();

    let reopen_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&reopen_called);
    let reopen: evo_ports::ReopenFn = Box::new(move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(MockDevice::new(ModelId::Evo4)))
    });

    let config = CoreConfig {
        reconnect: ReconnectPolicy {
            enabled: false,
            ..ReconnectPolicy::default()
        },
        ..CoreConfig::default()
    };

    let core = EvoCore::spawn_with_reopen(Box::new(first), Some(reopen), config);
    let mut connection = core.subscribe_connection();

    *faults.lock().expect("mutex") = Some(Fault::Disconnect);
    core.submit(DeviceCommand::SetGain { ch: 0, db: 1.0 })
        .await
        .expect("aceptado");

    let state = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let current = *connection.borrow_and_update();
            if current == ConnectionState::Disconnected {
                return current;
            }
            if connection.changed().await.is_err() {
                panic!("el núcleo se apagó");
            }
        }
    })
    .await
    .expect("debe quedar desconectado");
    assert_eq!(state, ConnectionState::Disconnected);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !reopen_called.load(std::sync::atomic::Ordering::SeqCst),
        "sin política habilitada no se debe intentar reabrir"
    );

    core.shutdown().expect("apagado limpio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn el_apagado_limpio_devuelve_el_dispositivo_con_lo_aplicado() {
    let device = MockDevice::new(ModelId::Evo4);
    // El registro es compartido: sobrevive a que el actor posea el dispositivo.
    let log = device.log();
    let core = EvoCore::spawn(Box::new(device), CoreConfig::default());

    core.submit(DeviceCommand::SetGain { ch: 1, db: 8.0 })
        .await
        .expect("aceptado");
    // Sin esperar al delta: el apagado debe aplicar lo pendiente antes de salir.
    let _device = core.shutdown().expect("apagado limpio");

    assert!(
        log.lock()
            .expect("mutex")
            .contains(&DeviceCommand::SetGain { ch: 1, db: 8.0 })
    );
}

#[test]
fn el_store_reemplaza_estado_solo_cuando_difiere() {
    let profile = DeviceProfile::evo4();
    let mut store = StateStore::initial(profile.clone());
    let mut other = DeviceState::initial(&profile);
    other.channels[0].gain_db = 20.0;
    other.loopback = true;

    let event = store.replace(other.clone()).expect("hay cambio");
    assert_eq!(event.rev, 1);
    assert_eq!(
        event.ops,
        vec![
            evo_domain::StateOp::Gain { ch: 0, db: 20.0 },
            evo_domain::StateOp::Loopback { on: true },
        ]
    );

    assert_eq!(store.replace(other), None, "sin cambio, sin revisión");
    assert_eq!(store.rev(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn los_eventos_reactivos_de_hardware_actualizan_el_store_inmediatamente() {
    let device = MockDevice::new(ModelId::Evo4);
    let trigger = device.hw_trigger();
    let core = EvoCore::spawn(Box::new(device), CoreConfig::default());
    let mut snapshots = core.subscribe_snapshots();
    let mut deltas = core.subscribe_state();

    // Simulamos un giro físico de perilla en el hardware (ganancia canal 0 sube a 36.5 dB)
    {
        let sink_guard = trigger.lock().expect("mutex");
        let sink = sink_guard.as_ref().expect("sink registrado por el actor");
        sink(DeviceEvent::GainChanged { ch: 0, db: 36.5 });
    }

    // El actor debe procesar el evento de inmediato sin polling ni comandos de software
    let snapshot = wait_for(&mut snapshots, "ganancia física actualizada", |s| {
        (s.state.channels[0].gain_db - 36.5).abs() < f32::EPSILON
    })
    .await;

    assert_eq!(snapshot.rev, 1);
    assert_eq!(snapshot.state.channels[0].gain_db, 36.5);

    // Verificamos que se emitió el delta por el bus hacia la UI
    let delta = tokio::time::timeout(Duration::from_millis(500), deltas.recv())
        .await
        .expect("delta recibido a tiempo")
        .expect("canal abierto");
    assert_eq!(delta.rev, 1);
    assert_eq!(
        delta.ops,
        vec![evo_domain::StateOp::Gain { ch: 0, db: 36.5 }]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn los_eventos_reactivos_idempotentes_o_ecos_no_generan_deltas_duplicados() {
    let device = MockDevice::new(ModelId::Evo4);
    let trigger = device.hw_trigger();
    let core = EvoCore::spawn(Box::new(device), CoreConfig::default());
    let mut snapshots = core.subscribe_snapshots();
    let mut deltas = core.subscribe_state();

    // 1) Disparamos evento físico
    {
        let sink_guard = trigger.lock().expect("mutex");
        let sink = sink_guard.as_ref().expect("sink registrado");
        sink(DeviceEvent::GainChanged { ch: 1, db: 22.0 });
    }

    wait_for(&mut snapshots, "ganancia canal 1 en 22.0", |s| {
        (s.state.channels[1].gain_db - 22.0).abs() < f32::EPSILON
    })
    .await;
    let _ = deltas.recv().await;

    // 2) Disparamos exactamente el mismo valor de nuevo (eco de ALSA)
    {
        let sink_guard = trigger.lock().expect("mutex");
        let sink = sink_guard.as_ref().expect("sink registrado");
        sink(DeviceEvent::GainChanged { ch: 1, db: 22.0 });
    }

    // No debe emitirse ningún nuevo delta (supresión de eco)
    let timed_out = tokio::time::timeout(Duration::from_millis(100), deltas.recv()).await;
    assert!(timed_out.is_err(), "no debe emitirse delta redundante ante un eco");

    assert_eq!(core.snapshot().rev, 1);
}

