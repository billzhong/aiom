//! AIO Monitor client.
//!
//! Samples the metrics of the host it runs on and reports them to the server over a
//! WebSocket. See [`aiom_protocol`] for the wire format.

mod args;
mod collector;
mod metrics;
pub mod session;
mod storage;
mod virt;

pub use crate::args::Args;
pub use crate::collector::Collector;

/// Installs the process-wide TLS crypto provider, unless one is installed already.
pub fn install_crypto_provider() {
    // Fails only when a provider is already installed, which is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
}
