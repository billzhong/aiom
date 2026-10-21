use clap::Parser;
use env_logger::Env;
use std::process::ExitCode;

use aiom_client::{Args, Collector, session};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = Args::parse();
    let default_filter = if args.debug {
        "info,aiom_client=debug"
    } else {
        "info"
    };
    env_logger::Builder::from_env(Env::default().default_filter_or(default_filter))
        .format_timestamp_millis()
        .init();
    log::debug!("Debug logging is enabled");

    aiom_client::install_crypto_provider();
    let config = match session::Config::new(
        &args.name,
        &args.server,
        &args.token,
        args.interval(),
        args.retry(),
    ) {
        Ok(config) => config,
        Err(e) => {
            log::error!("Invalid server URL: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut collector = Collector::new();
    session::run(&config, &mut collector, shutdown_signal()).await;
    ExitCode::SUCCESS
}

/// Completes on Ctrl-C, or on SIGTERM as sent by `systemctl stop`.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            log::error!("Listening for Ctrl-C failed: {e}");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(e) => {
                log::error!("Listening for SIGTERM failed: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
