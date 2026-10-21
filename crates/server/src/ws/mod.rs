use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use bytes::Bytes;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::{Instant, Interval, MissedTickBehavior, timeout, timeout_at};

use crate::AppState;

pub(crate) mod agent;
pub(crate) mod dashboard;

/// How open WebSockets are kept in check.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keepalive {
    /// How often peers are pinged. A client that went away without closing its connection
    /// answers with a TCP reset as soon as it is reachable again, so when it reconnects after a
    /// network outage, its old connection closes and frees its name within this interval.
    /// Browsers answer pings by themselves, even for background tabs whose timers are
    /// throttled, so a live dashboard stays connected while it does not poll.
    pub(crate) ping_interval: Duration,
    /// How long a connection may stay silent, pongs included, before it is dropped.
    pub(crate) idle_timeout: Duration,
}

impl Default for Keepalive {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(20),
        }
    }
}

/// An open WebSocket, closed when the server shuts down.
#[derive(Debug)]
pub(crate) struct Connection {
    socket: WebSocket,
    shutdown: watch::Receiver<bool>,
    ping: Interval,
    idle_timeout: Duration,
}

impl Connection {
    pub(crate) fn new(socket: WebSocket, state: &AppState) -> Self {
        let Keepalive {
            ping_interval,
            idle_timeout,
        } = state.keepalive;
        let mut ping = tokio::time::interval_at(Instant::now() + ping_interval, ping_interval);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            socket,
            shutdown: state.shutdown.subscribe(),
            ping,
            idle_timeout,
        }
    }

    /// Waits for the next data message. Returns `None` when the connection ends or goes
    /// silent, or once the server shuts down, after telling the peer that it is going away.
    pub(crate) async fn recv(&mut self) -> Option<Message> {
        let mut deadline = Instant::now() + self.idle_timeout;
        loop {
            tokio::select! {
                message = timeout_at(deadline, self.socket.recv()) => match message {
                    Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {
                        deadline = Instant::now() + self.idle_timeout;
                    }
                    Ok(Some(Ok(Message::Close(_))) | None) => return None,
                    Ok(Some(Ok(message))) => return Some(message),
                    Ok(Some(Err(e))) => {
                        tracing::debug!("WebSocket error: {e}");
                        return None;
                    }
                    Err(_) => {
                        tracing::debug!("dropping silent WebSocket");
                        return None;
                    }
                },
                _ = self.ping.tick() => {
                    if !self.send(Message::Ping(Bytes::new())).await {
                        return None;
                    }
                }
                () = shut_down(&mut self.shutdown) => {
                    self.close(close_code::AWAY, "server shutting down").await;
                    return None;
                }
            }
        }
    }

    /// Sends `message`; returns whether it went out. Gives up on a peer that has not taken it
    /// within the idle timeout, so one that never reads cannot hold up the server.
    pub(crate) async fn send(&mut self, message: Message) -> bool {
        match timeout(self.idle_timeout, self.socket.send(message)).await {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                tracing::debug!("WebSocket error: {e}");
                false
            }
            Err(_) => {
                tracing::debug!("dropping WebSocket that does not read");
                false
            }
        }
    }

    /// Closes the connection with a close frame; the peer may already be gone.
    pub(crate) async fn close(&mut self, code: u16, reason: &'static str) {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        self.send(Message::Close(Some(frame))).await;
    }
}

/// Completes once the server shuts down.
async fn shut_down(shutdown: &mut watch::Receiver<bool>) {
    // Errs only when the sender is gone, which the state itself prevents.
    let _ = shutdown.wait_for(|&stop| stop).await;
}
