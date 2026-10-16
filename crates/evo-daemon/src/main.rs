//! Punto de entrada del binario `evo-daemon` (SAD §8.3).

use std::process::ExitCode;

use evo_daemon::cli::{self, Args, HELP};
use evo_daemon::{Daemon, DaemonConfig};
use evo_domain::ServerConfig;
use tracing::Level;

fn main() -> ExitCode {
    let mut args_iter = std::env::args();
    // El primer argumento es el nombre del programa.
    let _prog = args_iter.next();
    let args: Vec<String> = args_iter.collect();

    let args = match cli::parse(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("evo-daemon: {err}");
            eprintln!();
            eprintln!("{HELP}");
            return ExitCode::FAILURE;
        }
    };

    if args.help {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.version {
        println!("evo-daemon {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    init_tracing(args.verbosity);

    #[tokio::main]
    async fn run(args: Args) -> ExitCode {
        let config = match build_config(args) {
            Ok(cfg) => cfg,
            Err(msg) => {
                tracing::error!("{msg}");
                eprintln!("evo-daemon: {msg}");
                return ExitCode::FAILURE;
            }
        };

        let daemon = match Daemon::start(config).await {
            Ok(daemon) => daemon,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo arrancar el daemon");
                eprintln!("evo-daemon: {err}");
                return ExitCode::FAILURE;
            }
        };

        tracing::info!(
            loopback = %daemon.loopback_addr(),
            lan = ?daemon.lan_addr(),
            "evo-daemon en marcha"
        );

        wait_for_signal().await;
        tracing::info!("recibida señal de apagado; cerrando servicios…");
        daemon.shutdown().await;
        tracing::info!("apagar completado");
        ExitCode::SUCCESS
    }

    run(args)
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .try_init();
}

fn build_config(args: Args) -> Result<DaemonConfig, String> {
    let mut config = DaemonConfig::default();
    if let Some(path) = args.config {
        config.state_path = path;
    }
    if let Some(model) = args.model {
        config.model = model;
    }
    if let Some(debounce) = args.debounce {
        config.debounce = debounce;
    }
    config.allow_public_peers = args.allow_public_peers;
    config.acquire_lock = !args.no_lock;

    if args.remote_interface.is_some() || args.remote_port.is_some() {
        config.server_override = Some(ServerConfig {
            enabled: args.remote_interface.is_some(),
            interface: args.remote_interface,
            port: args.remote_port.unwrap_or(8080),
        });
    }

    Ok(config)
}

async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) {
        (Ok(mut term), Ok(mut int)) => {
            tokio::select! {
                _ = term.recv() => {},
                _ = int.recv() => {},
            }
        }
        _ => {
            tracing::warn!(
                "no se pudieron registrar los manejadores de SIGTERM/SIGINT; el daemon no se apagará por señal"
            );
            std::future::pending::<()>().await
        }
    }
}
