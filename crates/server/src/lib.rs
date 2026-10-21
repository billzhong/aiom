//! AIO Monitor server.
//!
//! Clients connect to the agent route and push reports; dashboards connect to the WebSocket
//! route and poll the node list, which is kept in memory. Optionally serves the dashboard
//! itself as well. See [`aiom_protocol`] for the wire format.

mod auth;
mod client_addr;
mod config;
mod geoip;
mod nodes;
mod snapshot;
mod web;
mod ws;

use anyhow::Context;
use axum::Router;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::get;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;

use crate::auth::BearerToken;
pub use crate::config::Config;
pub use crate::geoip::GeoIp;
use crate::nodes::Nodes;
use crate::snapshot::SnapshotCache;
use crate::ws::Keepalive;

/// Route answering `200 OK` while the server is up.
pub const HEALTH_PATH: &str = "/healthz";

/// Dashboards poll about once a second; serve them all from one encoded node list.
const DEFAULT_SNAPSHOT_TTL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    nodes: Arc<Nodes>,
    geoip: GeoIp,
    snapshots: Arc<SnapshotCache>,
    keepalive: Keepalive,
    /// Set to `true` when the server shuts down. Open WebSockets are not tracked by the HTTP
    /// server's graceful shutdown, so they watch this to close themselves.
    shutdown: watch::Sender<bool>,
}

/// A configured server, ready to [`serve`](App::serve).
#[derive(Debug)]
pub struct App {
    state: AppState,
    token: BearerToken,
    dashboard_path: String,
    agent_path: String,
    web: Option<web::Source>,
}

impl App {
    /// Validates `config`.
    pub fn new(config: &Config, geoip: GeoIp) -> anyhow::Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        let web = web::Source::new(config.web_dir())?;
        Ok(Self {
            state: AppState {
                nodes: Arc::default(),
                geoip,
                snapshots: Arc::new(SnapshotCache::new(DEFAULT_SNAPSHOT_TTL)),
                keepalive: Keepalive::default(),
                shutdown: watch::channel(false).0,
            },
            token: BearerToken::new(&config.token),
            dashboard_path: config.dashboard_path.clone(),
            agent_path: config.agent_path.clone(),
            web,
        })
    }

    /// Sets how long dashboards may be served the same node list.
    #[must_use]
    pub fn with_snapshot_ttl(mut self, ttl: Duration) -> Self {
        self.state.snapshots = Arc::new(SnapshotCache::new(ttl));
        self
    }

    /// Sets how often WebSocket peers are pinged, and how long a connection may stay silent
    /// before it is dropped.
    #[must_use]
    pub fn with_keepalive(mut self, ping_interval: Duration, idle_timeout: Duration) -> Self {
        self.state.keepalive = Keepalive {
            ping_interval,
            idle_timeout,
        };
        self
    }

    fn router(&self) -> Router {
        let agent = get(ws::agent::upgrade).route_layer(middleware::from_fn_with_state(
            self.token.clone(),
            auth::require_bearer,
        ));
        let router = Router::new()
            .route(&self.dashboard_path, get(ws::dashboard::upgrade))
            .route(&self.agent_path, agent)
            .route(HEALTH_PATH, get(|| async { "ok" }));
        let router = match &self.web {
            Some(source) => router.fallback_service(web::router(source)),
            None => router.fallback(|| async { StatusCode::NOT_FOUND }),
        };
        router
            .layer(TraceLayer::new_for_http())
            .with_state(self.state.clone())
    }

    /// Serves requests on `listener` until `shutdown` completes, then closes open WebSockets.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let notify = self.state.shutdown.clone();
        // The peer address locates clients that are not behind a proxy.
        let service = self
            .router()
            .into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, service)
            .with_graceful_shutdown(async move {
                shutdown.await;
                notify.send_replace(true);
            })
            .await
    }
}

/// Runs the server described by `config` until SIGINT or SIGTERM.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let geoip = GeoIp::ipinfo(config.ipinfo_token().map(str::to_owned))?;
    let app = App::new(&config, geoip)?;
    let listener = TcpListener::bind(&config.listen)
        .await
        .with_context(|| format!("failed to listen on {}", config.listen))?;
    tracing::info!(
        address = %listener.local_addr()?,
        agent_path = %config.agent_path,
        dashboard_path = %config.dashboard_path,
        web = app.web.as_ref().map(tracing::field::display),
        "listening"
    );
    app.serve(listener, shutdown_signal()).await?;
    tracing::info!("shut down");
    Ok(())
}

/// Installs the process-wide TLS crypto provider, unless one is installed already.
pub(crate) fn install_crypto_provider() {
    // Fails only when a provider is already installed, which is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to listen for Ctrl-C: {e}");
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
                tracing::error!("failed to listen for SIGTERM: {e}");
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
    tracing::info!("shutting down");
}
