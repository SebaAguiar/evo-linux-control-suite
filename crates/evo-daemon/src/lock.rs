//! Instancia única del daemon mediante un fichero de bloqueo (SAD §8.3).
//!
//! Solo puede haber **un** dueño del dispositivo: el daemon. Un segundo arranque
//! debe negarse en lugar de competir por el USB. El SAD §8.3 pide un *lock file*
//! en `$XDG_RUNTIME_DIR`; aquí se materializa con `O_EXCL` y el PID del proceso,
//! sin dependencias externas.
//!
//! Frente a `flock`, esta variante no necesita `libc` y es portable a cualquier
//! sistema de ficheros. El precio es que un cierre abrupto deja el fichero: por eso
//! al arrancar se comprueba si el PID sigue vivo (`/proc/<pid>`) y, si no, el
//! bloqueo se considera huérfano y se retira.

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// Error al adquirir el bloqueo de instancia única.
#[derive(Debug, Error)]
pub enum LockError {
    /// Ya hay un daemon en ejecución con ese PID.
    #[error("ya hay un evo-daemon en ejecución (pid {pid})")]
    AlreadyRunning {
        /// PID leído del fichero de bloqueo.
        pid: u32,
    },
    /// No se pudo crear o leer el fichero de bloqueo.
    #[error("no se pudo operar sobre el bloqueo `{path}`: {source}")]
    Io {
        /// Ruta del fichero de bloqueo.
        path: PathBuf,
        /// Causa del sistema.
        #[source]
        source: std::io::Error,
    },
}

/// Bloqueo de instancia única. Se libera al soltar el valor (o al morir el
/// proceso, porque el fichero queda huérfano y el siguiente arranque lo detecta).
#[derive(Debug)]
pub struct SingleInstance {
    path: PathBuf,
}

impl SingleInstance {
    /// Intenta tomar el bloqueo en `path`.
    ///
    /// Si el fichero existe pero su PID no está vivo, se trata como huérfano y se
    /// reintenta una vez.
    pub fn acquire(path: impl Into<PathBuf>) -> Result<Self, LockError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    // Escribir el PID permite al siguiente arranque distinguir un
                    // bloqueo vivo de uno huérfano.
                    let _ = writeln!(file, "{}", std::process::id());
                    let _ = file.sync_all();
                    return Ok(Self { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    match read_pid_settled(&path) {
                        Some(pid) if process_alive(pid) => {
                            return Err(LockError::AlreadyRunning { pid });
                        }
                        // Sin PID legible o con PID muerto: huérfano, se retira.
                        _ => {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
                Err(err) => {
                    return Err(LockError::Io { path, source: err });
                }
            }
        }

        // La segunda vuelta no debería llegar aquí; si pasa, hay contención real.
        Err(LockError::AlreadyRunning {
            pid: read_pid(&path).unwrap_or(0),
        })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Lee el PID del fichero de bloqueo, si es legible.
fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Lee el PID dando una pequeña tregua a un proceso que acaba de crear el
/// fichero y aún no ha escrito su PID.
fn read_pid_settled(path: &Path) -> Option<u32> {
    if let Some(pid) = read_pid(path) {
        return Some(pid);
    }
    std::thread::sleep(Duration::from_millis(50));
    read_pid(path)
}

/// ¿Existe el proceso? En Linux basta con que esté su directorio en `/proc`.
fn process_alive(pid: u32) -> bool {
    pid != 0 && Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_segundo_bloqueo_del_mismo_proceso_se_rechaza() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let path = dir.path().join("evo-daemon.lock");

        let first = SingleInstance::acquire(&path).expect("primer bloqueo");
        let second = SingleInstance::acquire(&path);
        assert!(
            matches!(second, Err(LockError::AlreadyRunning { .. })),
            "un segundo daemon no debe arrancar: {second:?}"
        );
        drop(first);
    }

    #[test]
    fn al_soltar_el_bloqueo_se_puede_volver_a_tomar() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let path = dir.path().join("evo-daemon.lock");

        drop(SingleInstance::acquire(&path).expect("primero"));
        SingleInstance::acquire(&path).expect("tras soltar, el bloqueo queda libre");
    }

    #[test]
    fn un_bloqueo_huerfano_se_retira() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let path = dir.path().join("evo-daemon.lock");
        // PID imposible de estar vivo: simula un daemon que murió sin limpiar.
        fs::write(&path, "4294967295\n").expect("dejar bloqueo huérfano");

        SingleInstance::acquire(&path).expect("debe retirar el huérfano y continuar");
    }
}
