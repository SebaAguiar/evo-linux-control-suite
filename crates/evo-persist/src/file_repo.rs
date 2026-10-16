//! Repositorio de estado en disco: TOML con escritura atómica (SAD §8.5).
//!
//! Dos cosas que este archivo debe hacer bien, porque son las que se descubren
//! cuando ya es tarde:
//!
//! - **Escritura atómica.** Un apagón a mitad de un `write` no puede dejar un
//!   `config.toml` truncado, porque el arranque lo leería y perdería el estado
//!   comandado. El orden es: escribir el temporal en el *mismo* directorio,
//!   `fsync`, `rename` (atómico en el mismo sistema de ficheros) y `fsync` del
//!   directorio para que la entrada sobreviva al corte.
//! - **Tolerancia a corrupción.** Si el documento está roto, se intenta el `.bak`.
//!   Si tampoco sirve, se degrada a "sin estado guardado" (que es controlable)
//!   en lugar de bloquear el arranque (SAD §8.5): el usuario debe poder seguir
//!   controlando su tarjeta aunque el archivo esté mal.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use evo_domain::{PersistedState, RepoError};
use evo_ports::StateRepo;

use crate::xdg;

/// Repositorio de estado basado en un único documento TOML.
#[derive(Debug, Clone)]
pub struct FileStateRepo {
    path: PathBuf,
}

impl FileStateRepo {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Repositorio en la ruta por defecto (`$XDG_CONFIG_HOME/evo-suite/config.toml`).
    pub fn default_path() -> Self {
        Self::new(xdg::config_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ruta de la copia de respaldo.
    fn backup_path(&self) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(".bak");
        self.path.with_file_name(name)
    }

    /// Ruta del fichero temporal de escritura.
    ///
    /// Lleva el PID para que dos procesos que arrancasen a la vez no se pisen el
    /// temporal. En v1.0 escribe un único daemon, pero no cuesta nada ser correcto.
    fn temp_path(&self) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".tmp-{}", std::process::id()));
        self.path.with_file_name(name)
    }

    /// Lee y deserializa un documento, si existe.
    fn read_doc(&self, path: &Path) -> Result<Option<PersistedState>, RepoError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(RepoError::Read(format!("{}: {err}", path.display())));
            }
        };

        let doc: PersistedState = toml::from_str(&text)
            .map_err(|err| RepoError::Corrupt(format!("{}: {err}", path.display())))?;

        if doc.schema_version > evo_domain::state::SCHEMA_VERSION {
            return Err(RepoError::Corrupt(format!(
                "schema_version {} es más nuevo que el soportado ({})",
                doc.schema_version,
                evo_domain::state::SCHEMA_VERSION
            )));
        }
        Ok(Some(doc))
    }
}

impl StateRepo for FileStateRepo {
    fn load(&self) -> Result<Option<PersistedState>, RepoError> {
        match self.read_doc(&self.path) {
            Ok(found) => Ok(found),
            Err(RepoError::Corrupt(detail)) => {
                // El documento principal está roto: se intenta el respaldo antes
                // de rendirse, y se avisa de lo ocurrido por el canal de log.
                tracing::warn!(%detail, "config.toml corrupto; se intenta el respaldo");
                match self.read_doc(&self.backup_path()) {
                    Ok(Some(doc)) => {
                        tracing::warn!("estado recuperado desde el respaldo .bak");
                        Ok(Some(doc))
                    }
                    Ok(None) => Ok(None),
                    Err(bak_err) => Err(RepoError::Corrupt(format!(
                        "principal y respaldo ilegibles ({detail}; {bak_err})"
                    ))),
                }
            }
            Err(other) => Err(other),
        }
    }

    fn save(&self, state: &PersistedState) -> Result<(), RepoError> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir).map_err(|err| {
            RepoError::Write(format!("no se pudo crear {}: {err}", dir.display()))
        })?;

        let text = toml::to_string(state)
            .map_err(|err| RepoError::Write(format!("no se pudo serializar: {err}")))?;

        // 1) Conservar la versión anterior como respaldo, antes de tocarla.
        if self.path.exists() {
            fs::copy(&self.path, self.backup_path()).map_err(|err| {
                RepoError::Write(format!("no se pudo escribir el respaldo: {err}"))
            })?;
        }

        // 2) Escribir el temporal y forzar a disco.
        let tmp = self.temp_path();
        {
            let mut file = fs::File::create(&tmp).map_err(|err| {
                RepoError::Write(format!("no se pudo crear {}: {err}", tmp.display()))
            })?;
            file.write_all(text.as_bytes())
                .map_err(|err| RepoError::Write(format!("escritura incompleta: {err}")))?;
            file.sync_all()
                .map_err(|err| RepoError::Write(format!("fsync del temporal falló: {err}")))?;
        }

        // 3) Renombrar: sustitución atómica en el mismo sistema de ficheros.
        fs::rename(&tmp, &self.path).map_err(|err| {
            let _ = fs::remove_file(&tmp);
            RepoError::Write(format!("rename falló: {err}"))
        })?;

        // 4) `fsync` del directorio para que la entrada del rename sobreviva a un
        //    corte de energía. En algunos sistemas de ficheros no está permitido
        //    abrir un directorio así, y no es motivo para fallar el guardado.
        if let Ok(dir_handle) = fs::File::open(dir) {
            let _ = dir_handle.sync_all();
        }

        Ok(())
    }
}
