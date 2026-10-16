//! Rutas XDG del proyecto (SAD §3.1).
//!
//! Se resuelven a mano, sin dependencias, porque la regla es simple y explícita:
//! `$XDG_*_HOME` si está definida y es absoluta, y si no el valor por defecto del
//! estándar. Un crate como `dirs` aporta mucho más de lo que aquí se necesita y
//! esconde justo las decisiones que interesa documentar.

use std::path::{Path, PathBuf};

/// Nombre del directorio de la aplicación dentro de las raíces XDG.
pub const APP_DIR: &str = "evo-suite";

/// Nombre del documento de configuración y estado.
pub const CONFIG_FILE: &str = "config.toml";

/// Resuelve una raíz XDG: la variable de entorno si es una ruta absoluta, si no
/// el valor por defecto relativo a `$HOME`.
fn base(env_var: &str, fallback: &str) -> PathBuf {
    if let Some(value) = std::env::var_os(env_var) {
        let path = PathBuf::from(value);
        // El estándar XDG exige rutas absolutas: una relativa es un error de
        // configuración del usuario y se ignora en lugar de usarse a medias.
        if path.is_absolute() {
            return path;
        }
    }
    home().join(fallback)
}

/// `$HOME`, o `/` si ni siquiera eso está definido.
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_CONFIG_HOME` (por defecto `~/.config`).
pub fn config_home() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME` (por defecto `~/.local/share`).
pub fn data_home() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_STATE_HOME` (por defecto `~/.local/state`).
pub fn state_home() -> PathBuf {
    base("XDG_STATE_HOME", ".local/state")
}

/// Directorio de configuración de la aplicación.
pub fn config_dir() -> PathBuf {
    config_home().join(APP_DIR)
}

/// Ruta del documento `config.toml` (estado comandado + ajustes).
pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE)
}

/// Ruta del documento dentro de un directorio concreto.
///
/// Útil para tests y para el modo *portable* (`--config-dir`).
pub fn config_path_in(dir: impl AsRef<Path>) -> PathBuf {
    dir.as_ref().join(CONFIG_FILE)
}
