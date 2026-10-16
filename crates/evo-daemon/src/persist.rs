//! Reaplicación del estado al arrancar y guardado con *debounce* (SAD §8.5, §8.6).
//!
//! Dos tareas distintas, ambas alrededor del mismo documento:
//!
//! - **Reaplicar.** Como el hardware no sabe decir en qué estado está (SAD §8.1),
//!   el daemon recuerda el último estado comandado y lo vuelve a enviar al
//!   arrancar, para que la tarjeta no aparezca con los faders en un valor que no
//!   es el que el usuario dejó. Ganancia y mute; el phantom **no** (SAD §8.5).
//! - **Guardar.** El daemon es el único escritor (SAD §8.5). Guarda con una
//!   ventana de *debounce* para no machacar el disco mientras se arrastra un
//!   fader, y reemite siempre el bloque `[server]` que cargó, para no borrarlo.

use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use evo_core::EvoCore;
use evo_domain::PersistedState;
use evo_domain::ServerConfig;
use evo_persist::FileStateRepo;
use evo_ports::StateRepo;
use tokio::sync::oneshot;

use crate::replay_commands;

/// Reaplica el último estado comandado sobre el dispositivo recién abierto.
///
/// Se omite si el modelo persistido no es el del dispositivo abierto: reaplicar
/// ganancias de un EVO 16 sobre un EVO 4 no tiene sentido y `SafetyPolicy` las
/// recortaría sin avisar.
pub(crate) async fn replay(core: &EvoCore, doc: &PersistedState) {
    let profile = core.snapshot().profile;
    if doc.profile.model != profile.model {
        tracing::info!(
            persisted = doc.profile.model.code(),
            device = profile.model.code(),
            "el estado guardado es de otro modelo; no se reaplica"
        );
        return;
    }

    let commands = replay_commands(doc, profile.inputs);
    let total = commands.len();
    for cmd in commands {
        if let Err(err) = core.submit(cmd).await {
            // Un comando rechazado (por ejemplo, un canal que el modelo no tiene)
            // no debe impedir reaplicar el resto.
            tracing::warn!(op = cmd.op_name(), error = %err, "no se pudo reaplicar un comando");
        }
    }
    tracing::info!(commands = total, "estado persistido reaplicado");
}

/// Guarda el documento con *debounce* hasta recibir la señal de parada.
///
/// En la parada, si había cambios sin escribir, hace un volcado final: el último
/// gesto del usuario no se pierde por apagar justo después.
pub(crate) async fn run(
    core: Arc<EvoCore>,
    repo: FileStateRepo,
    server: ServerConfig,
    debounce: Duration,
    mut stop: oneshot::Receiver<()>,
) {
    let mut snapshots = core.subscribe_snapshots();
    let mut _dirty = false;

    loop {
        tokio::select! {
            _ = &mut stop => {
                if _dirty {
                    save(&core, &repo, &server);
                }
                return;
            }
            changed = snapshots.changed() => {
                if changed.is_err() {
                    // El núcleo se apagó: nada que guardar.
                    return;
                }
                _dirty = true;
                // Esperar quietud: cada cambio nuevo reinicia la ventana.
                loop {
                    tokio::select! {
                        _ = &mut stop => {
                            save(&core, &repo, &server);
                            return;
                        }
                        _ = tokio::time::sleep(debounce) => break,
                        again = snapshots.changed() => {
                            if again.is_err() {
                                return;
                            }
                            // sigue sucio: la ventana se reinicia
                        }
                    }
                }
                save(&core, &repo, &server);
                _dirty = false;
            }
        }
    }
}

/// Escribe el documento, reemitiendo los ajustes `[server]` cargados.
fn save(core: &EvoCore, repo: &FileStateRepo, server: &ServerConfig) {
    let snapshot = core.snapshot();
    let doc =
        PersistedState::new(snapshot.profile, snapshot.state, now_ms()).with_server(server.clone());
    if let Err(err) = repo.save(&doc) {
        tracing::warn!(error = %err, "no se pudo guardar el estado");
    }
}

/// Marca temporal del documento, en milisegundos desde la época.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
