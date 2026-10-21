//! The channel clients report on.

use aiom_protocol::{AgentData, AgentReport, PROTOCOL_VERSION, decode, unix_now};
use axum::Extension;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::Response;
use std::net::{IpAddr, SocketAddr};
use tracing::field::display;

use super::Connection;
use crate::AppState;
use crate::client_addr::public_addr;
use crate::nodes::Claim;

/// Reports are around 1 KiB compressed; anything far larger is not a report.
const MAX_MESSAGE_SIZE: usize = 64 * 1024;
const MAX_REPORT_SIZE: usize = 256 * 1024;

pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
) -> Response {
    let addr = public_addr(&headers, peer.map(|Extension(ConnectInfo(peer))| peer.ip()));
    ws.max_message_size(MAX_MESSAGE_SIZE)
        .max_frame_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle(socket, state, addr))
}

/// Stores the reports of one client connecting from the public address `addr`, if known.
async fn handle(socket: WebSocket, state: AppState, addr: Option<IpAddr>) {
    let mut connection = Connection::new(socket, &state);
    // Declared after the connection, so the name is free again before the socket closes.
    let mut claim: Option<Claim> = None;
    while let Some(message) = connection.recv().await {
        let Message::Binary(payload) = message else {
            continue;
        };
        let report = match decode::<AgentReport>(&payload, MAX_REPORT_SIZE) {
            Ok(report) if report.version == PROTOCOL_VERSION => report,
            Ok(report) => {
                tracing::warn!(version = report.version, "unsupported protocol version");
                connection
                    .close(close_code::PROTOCOL, "unsupported protocol version")
                    .await;
                return;
            }
            Err(e) => {
                let node = claim.as_ref().map(|claim| display(claim.name()));
                tracing::warn!(node, "malformed report: {e}");
                connection
                    .close(close_code::INVALID, "malformed report")
                    .await;
                return;
            }
        };
        let AgentData {
            name,
            host,
            stats,
            ip,
        } = report.data;

        let claim = match &mut claim {
            Some(claim) if claim.name() == name => claim,
            _ => {
                // Release the name this connection reported as so far, if any.
                claim = None;
                let Some(new) = state.nodes.claim(&name) else {
                    tracing::warn!(node = %name, "rejecting agent: another client reports as this node");
                    connection
                        .close(close_code::POLICY, "another client reports as this node")
                        .await;
                    return;
                };
                tracing::info!(node = %name, address = addr.map(display), "agent connected");
                claim.insert(new)
            }
        };

        // Geolocate the public address the client connects from, else one it reported.
        let geoip = match addr.or_else(|| ip.preferred()) {
            Some(ip) => Some(state.geoip.country_code(ip).await.unwrap_or_default()),
            None => None,
        };
        claim.update(unix_now(), host, stats, geoip);
    }
    if let Some(claim) = claim {
        tracing::info!(node = %claim.name(), "agent disconnected");
    }
}
