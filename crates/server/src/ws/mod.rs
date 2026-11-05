use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use bytes::Bytes;
use std::convert::Infallible;
use std::future::{Future, pending};
use std::pin::pin;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::{Instant, Interval, MissedTickBehavior, timeout, timeout_at};

use crate::AppState;

pub(crate) mod agent;
pub(crate) mod dashboard;

/// How long closing waits for the peer to complete the closing handshake.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

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

/// What [`Connection::recv_or`] waited for.
#[derive(Debug)]
pub(crate) enum Event<T> {
    Message(Message),
    /// The other future completed first.
    Done(T),
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
        let Event::Message(message) = self.recv_or(pending::<Infallible>()).await?;
        Some(message)
    }

    /// Like [`recv`](Self::recv), but also returns once `other` completes, while the
    /// connection is kept in check as before.
    pub(crate) async fn recv_or<T>(&mut self, other: impl Future<Output = T>) -> Option<Event<T>> {
        let mut other = pin!(other);
        let mut deadline = Instant::now() + self.idle_timeout;
        loop {
            tokio::select! {
                message = timeout_at(deadline, self.socket.recv()) => match message {
                    Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {
                        deadline = Instant::now() + self.idle_timeout;
                    }
                    Ok(Some(Ok(Message::Close(_)))) => {
                        // The reply is only queued so far.
                        self.finish_close().await;
                        return None;
                    }
                    Ok(None) => return None,
                    Ok(Some(Ok(message))) => return Some(Event::Message(message)),
                    Ok(Some(Err(e))) => {
                        tracing::debug!("WebSocket error: {e}");
                        return None;
                    }
                    Err(_) => {
                        tracing::debug!("dropping silent WebSocket");
                        return None;
                    }
                },
                value = &mut other => return Some(Event::Done(value)),
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
        if self.send(Message::Close(Some(frame))).await {
            self.finish_close().await;
        }
    }

    /// Completes the closing handshake: sends the reply to the close frame of the peer, which
    /// the socket sends on its next use, or waits for the reply to ours. Gives up on a peer that
    /// does not answer within [`CLOSE_TIMEOUT`].
    async fn finish_close(&mut self) {
        let _ = timeout(CLOSE_TIMEOUT, async {
            while let Some(Ok(_)) = self.socket.recv().await {}
        })
        .await;
    }
}

/// Completes once the server shuts down.
async fn shut_down(shutdown: &mut watch::Receiver<bool>) {
    // Errs only when the sender is gone, which the state itself prevents.
    let _ = shutdown.wait_for(|&stop| stop).await;
}
