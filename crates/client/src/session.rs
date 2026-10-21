//! The connection to the server.

use aiom_protocol::{AgentData, AgentReport, PROTOCOL_VERSION, encode, unix_now};
use futures_util::{SinkExt, StreamExt};
use std::fmt::Display;
use std::future::Future;
use std::pin::{Pin, pin};
use std::time::Duration;
use tokio::time::{MissedTickBehavior, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};
use url::Url;

use crate::collector::Collector;

/// How and where to report.
#[derive(Debug, Clone)]
pub struct Config {
    name: String,
    request: Request,
    interval: Duration,
    retry: Duration,
}

impl Config {
    /// Reports as `name` to the agent URL `server` every `interval`, authenticating with
    /// `token`. After a failure, reconnects after `retry`, which also bounds how long
    /// connecting and sending may take.
    pub fn new(
        name: &str,
        server: &Url,
        token: &str,
        interval: Duration,
        retry: Duration,
    ) -> Result<Self, tungstenite::Error> {
        let mut request = server.as_str().into_client_request()?;
        let authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| tungstenite::Error::HttpFormat(e.into()))?;
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, authorization);
        Ok(Self {
            name: name.to_owned(),
            request,
            interval,
            retry,
        })
    }
}

enum Outcome {
    Disconnected,
    Shutdown,
}

/// Reports to the server until `shutdown` completes, reconnecting whenever the connection
/// fails.
pub async fn run(config: &Config, collector: &mut Collector, shutdown: impl Future<Output = ()>) {
    let mut shutdown = pin!(shutdown);
    loop {
        if let Outcome::Shutdown = session(config, collector, shutdown.as_mut()).await {
            return;
        }
        log::debug!("Reconnecting in {} s", config.retry.as_secs_f32());
        tokio::select! {
            () = &mut shutdown => return,
            () = tokio::time::sleep(config.retry) => {}
        }
    }
}

/// Connects and reports until the connection fails or `shutdown` completes.
async fn session<F: Future<Output = ()>>(
    config: &Config,
    collector: &mut Collector,
    mut shutdown: Pin<&mut F>,
) -> Outcome {
    log::debug!("Connecting to {}", config.request.uri());
    let connect = within(
        config.retry,
        "connection",
        connect_async(config.request.clone()),
    );
    let socket = tokio::select! {
        () = &mut shutdown => return Outcome::Shutdown,
        result = connect => match result {
            Some((socket, _)) => socket,
            None => return Outcome::Disconnected,
        },
    };
    log::info!("WebSocket connection established");

    let (mut write, mut read) = socket.split();
    let mut interval = tokio::time::interval(config.interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let Some(report) = build_report(&config.name, collector).await else {
                    continue;
                };
                if within(config.retry, "send", write.send(Message::binary(report))).await.is_none() {
                    return Outcome::Disconnected;
                }
                log::debug!("Report sent");
            }
            // Reading keeps ping/pong flowing and notices a closed connection early.
            message = read.next() => {
                match message {
                    Some(Ok(Message::Close(frame))) => log_close(frame.as_ref()),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => log::error!("WebSocket receive error: {e}"),
                    None => log::info!("WebSocket connection ended"),
                }
                return Outcome::Disconnected;
            }
            () = &mut shutdown => {
                if within(config.retry, "close", write.send(Message::Close(None))).await.is_some() {
                    log::info!("WebSocket connection closed");
                }
                return Outcome::Shutdown;
            }
        }
    }
}

/// Runs the WebSocket operation `what` for at most `limit`, logging why it failed.
async fn within<T, E: Display>(
    limit: Duration,
    what: &str,
    operation: impl Future<Output = Result<T, E>>,
) -> Option<T> {
    match timeout(limit, operation).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(e)) => {
            log::error!("WebSocket {what} error: {e}");
            None
        }
        Err(_) => {
            log::error!("WebSocket {what} timed out");
            None
        }
    }
}

/// Logs why the server closed the connection: on purpose when it shuts down, while any
/// other code means it refused this client's reports.
fn log_close(frame: Option<&CloseFrame>) {
    match frame {
        Some(frame) if !matches!(frame.code, CloseCode::Normal | CloseCode::Away) => {
            log::error!(
                "Server refused the reports ({}): {}",
                u16::from(frame.code),
                frame.reason
            );
        }
        _ => log::info!("WebSocket closed by server"),
    }
}

async fn build_report(name: &str, collector: &mut Collector) -> Option<Vec<u8>> {
    let stats = collector.sample().await;
    let report = AgentReport {
        version: PROTOCOL_VERSION,
        timestamp: unix_now(),
        data: AgentData {
            name: name.to_owned(),
            host: collector.host().clone(),
            stats,
            ip: collector.public_ip(),
        },
    };
    encode(&report)
        .inspect_err(|e| log::error!("Encoding report failed: {e}"))
        .ok()
}
