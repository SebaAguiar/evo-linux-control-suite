//! Interpretación de la línea de órdenes del daemon.
//!
//! Se analiza a mano, sin `clap`: son pocas opciones, todas de una sola letra o
//! con valor único, y así el binario no arrastra un árbol de dependencias solo
//! para leer `argv`.

use std::path::PathBuf;
use std::time::Duration;

use evo_domain::ModelId;

/// Órdenes reconocidas en `argv`.
#[derive(Debug, Default, Clone)]
pub struct Args {
    /// Ruta del documento `config.toml` (por defecto, la XDG).
    pub config: Option<PathBuf>,
    /// Modelo a controlar.
    pub model: Option<ModelId>,
    /// Interfaz de LAN forzada (activa el servidor remoto).
    pub remote_interface: Option<String>,
    /// Puerto del servidor remoto forzado.
    pub remote_port: Option<u16>,
    /// No filtrar IP de origen públicas (SAD §8.4).
    pub allow_public_peers: bool,
    /// Ventana de *debounce* del guardado.
    pub debounce: Option<Duration>,
    /// No tomar el bloqueo de instancia única.
    pub no_lock: bool,
    /// Nivel de detalle: 0 = info, 1 = debug, ≥2 = trace.
    pub verbosity: u8,
    /// Se pidió la ayuda.
    pub help: bool,
    /// Se pidió la versión.
    pub version: bool,
}

/// Error de análisis de la línea de órdenes.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    /// Una opción con valor se quedó sin él.
    #[error("falta el valor de `{0}`")]
    MissingValue(String),
    /// El valor no es válido para esa opción.
    #[error("valor no válido para `{flag}`: `{value}`")]
    Invalid {
        /// Opción afectada.
        flag: String,
        /// Valor rechazado.
        value: String,
    },
    /// Opción no reconocida.
    #[error("opción desconocida: `{0}`")]
    Unknown(String),
}

/// Analiza `argv` (sin el nombre del programa).
pub fn parse<I>(args: I) -> Result<Args, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut out = Args::default();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        // Separa `--flag=valor` en `--flag` y `valor`.
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (arg.clone(), None),
        };

        // Devuelve el valor de una opción que lo exige (en línea o el siguiente).
        let mut take_value = |flag: &str| -> Result<String, CliError> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            iter.next()
                .ok_or_else(|| CliError::MissingValue(flag.to_string()))
        };

        match flag.as_str() {
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            "--allow-public-peers" => out.allow_public_peers = true,
            "--no-lock" => out.no_lock = true,
            "-v" => out.verbosity = out.verbosity.saturating_add(1),
            "--config" => out.config = Some(PathBuf::from(take_value("--config")?)),
            "--model" => {
                let value = take_value("--model")?;
                out.model = Some(parse_model(&value).ok_or_else(|| CliError::Invalid {
                    flag: "--model".to_string(),
                    value,
                })?);
            }
            "--remote-interface" => {
                out.remote_interface = Some(take_value("--remote-interface")?);
            }
            "--remote-port" => {
                let value = take_value("--remote-port")?;
                out.remote_port = Some(value.parse().map_err(|_| CliError::Invalid {
                    flag: "--remote-port".to_string(),
                    value,
                })?);
            }
            "--debounce-ms" => {
                let value = take_value("--debounce-ms")?;
                let ms: u64 = value.parse().map_err(|_| CliError::Invalid {
                    flag: "--debounce-ms".to_string(),
                    value,
                })?;
                out.debounce = Some(Duration::from_millis(ms));
            }
            "-vv" => out.verbosity = out.verbosity.saturating_add(2),
            other => return Err(CliError::Unknown(other.to_string())),
        }
    }

    Ok(out)
}

/// Traduce el código estable de modelo (`'evo4' | 'evo8' | 'evo16'`, SAD §3.2).
fn parse_model(value: &str) -> Option<ModelId> {
    ModelId::ALL.into_iter().find(|m| m.code() == value)
}

/// Texto de ayuda del binario.
pub const HELP: &str = "\
evo-daemon — control de interfaces Audient EVO sin interfaz gráfica (SAD §8.3)

USO:
    evo-daemon [OPCIONES]

OPCIONES:
    --config <RUTA>            Documento config.toml (por defecto, el XDG)
    --model <ev4|ev8|ev16>     Modelo a controlar (por defecto, evo4)
    --remote-interface <IF>    Interfaz de LAN; activa el servidor remoto
    --remote-port <PUERTO>     Puerto del servidor remoto
    --allow-public-peers       No filtrar orígenes públicos (SAD §8.4)
    --debounce-ms <MS>         Ventana de guardado de estado (por defecto, 2000)
    --no-lock                  No tomar el bloqueo de instancia única
    -v                         Registro detallado (repetible: -vv)
    -h, --help                 Muestra esta ayuda
    -V, --version              Muestra la versión";

/// Códigos de modelo aceptados por `--model`, para mensajes de error.
pub fn model_codes() -> impl Iterator<Item = &'static str> {
    ModelId::ALL.into_iter().map(ModelId::code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> Args {
        parse(args.iter().map(|s| s.to_string())).expect("args válidos")
    }

    #[test]
    fn un_argv_vacio_usa_los_valores_por_defecto() {
        let args = parse_ok(&[]);
        assert_eq!(args.model, None);
        assert!(!args.allow_public_peers);
        assert!(!args.no_lock);
        assert_eq!(args.verbosity, 0);
    }

    #[test]
    fn lee_opciones_con_valor_en_linea_y_separadas() {
        assert_eq!(parse_ok(&["--model", "evo8"]).model, Some(ModelId::Evo8));
        assert_eq!(parse_ok(&["--model=evo16"]).model, Some(ModelId::Evo16));
        assert_eq!(parse_ok(&["--remote-port=9000"]).remote_port, Some(9000));
        assert_eq!(
            parse_ok(&["--remote-interface", "wlan0"]).remote_interface,
            Some("wlan0".to_string())
        );
    }

    #[test]
    fn acumula_la_verbosidad() {
        assert_eq!(parse_ok(&["-v"]).verbosity, 1);
        assert_eq!(parse_ok(&["-v", "-v"]).verbosity, 2);
        assert_eq!(parse_ok(&["-vv"]).verbosity, 2);
    }

    #[test]
    fn rechaza_un_modelo_desconocido() {
        let err = parse(["--model".to_string(), "evo2".to_string()]).unwrap_err();
        assert!(matches!(err, CliError::Invalid { .. }), "{err:?}");
    }

    #[test]
    fn rechaza_un_puerto_no_numerico() {
        let err = parse(["--remote-port".to_string(), "ochenta".to_string()]).unwrap_err();
        assert!(matches!(err, CliError::Invalid { .. }), "{err:?}");
    }

    #[test]
    fn exige_el_valor_de_las_opciones_que_lo_llevan() {
        let err = parse(["--config".to_string()]).unwrap_err();
        assert_eq!(err, CliError::MissingValue("--config".to_string()));
    }

    #[test]
    fn rechaza_una_opcion_desconocida() {
        let err = parse(["--turbo".to_string()]).unwrap_err();
        assert!(matches!(err, CliError::Unknown(_)), "{err:?}");
    }
}
