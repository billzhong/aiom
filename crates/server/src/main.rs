use clap::Parser;
use std::io::IsTerminal;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

use aiom_server::Config;

#[tokio::main]
async fn main() -> ExitCode {
    // Load `.env` before reading RUST_LOG and the configuration from the environment.
    let dotenv = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // No color codes in log files and container logs.
        .with_ansi(std::io::stdout().is_terminal())
        .init();
    if let Err(e) = dotenv
        && !e.not_found()
    {
        tracing::warn!("ignoring .env: {e}");
    }

    let config = Config::parse();
    match aiom_server::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}
