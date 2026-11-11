//! The channel clients report on.

use aiom_protocol::{AgentData, AgentReport, ReportError, parse_name, unix_now};
use axum::Extension;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::Response;
use std::future::pending;
use std::net::{IpAddr, SocketAddr};
use tokio::task::JoinHandle;
use tracing::field::display;

use super::{Connection, Event};
use crate::client_addr::public_addr;
use crate::nodes::Claim;
use crate::{AppState, GeoIp};

/// Reports are around 1 KiB compressed; anything far larger is not a report.
const MAX_MESSAGE_SIZE: usize = 64 * 1024;
const MAX_REPORT_SIZE: usize = 256 * 1024;

/// A country lookup running in the background, and the address it looks up.
type Lookup = (IpAddr, JoinHandle<Option<String>>);

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
    // Looking a country up can take seconds, which must not hold up the reports.
    let mut lookup: Option<Lookup> = None;
    // The close code and reason the client is refused with, if it is.
    let refusal = loop {
        let message = match connection.recv_or(looked_up(&mut lookup)).await {
            Some(Event::Message(message)) => message,
            Some(Event::Done(country)) => {
                if let (Some(claim), Some(country)) = (&claim, country) {
                    claim.set_geoip(country);
                }
                continue;
            }
            None => break None,
        };
        let Message::Binary(payload) = message else {
            continue;
        };
        let report = match AgentReport::decode(&payload, MAX_REPORT_SIZE) {
            Ok(report) => report,
            Err(ReportError::UnsupportedVersion(version)) => {
                tracing::warn!(version, "unsupported protocol version");
                break Some((close_code::PROTOCOL, "unsupported protocol version"));
            }
            Err(ReportError::Codec(e)) => {
                let node = claim.as_ref().map(|claim| display(claim.name()));
                tracing::warn!(node, "malformed report: {e}");
                break Some((close_code::INVALID, "malformed report"));
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
                // Release the name this connection reported as so far, if any, and the
                // lookup for it.
                claim = None;
                lookup = None;
                // Refuse names the client does not send: one with surrounding whitespace would
                // look like its trimmed form on the dashboard, yet not conflict with it.
                if !parse_name(&name).is_ok_and(|parsed| parsed == name) {
                    tracing::warn!(node = ?name, "rejecting agent: invalid node name");
                    break Some((close_code::INVALID, "invalid node name"));
                }
                let Some(new) = state.nodes.claim(&name) else {
                    tracing::warn!(node = %name, "rejecting agent: another client reports as this node");
                    break Some((close_code::POLICY, "another client reports as this node"));
                };
                tracing::info!(node = %name, address = addr.map(display), "agent connected");
                claim.insert(new)
            }
        };

        // Geolocate the public address the client connects from, else one it reported. A
        // country not found keeps the one known before.
        let geoip = addr
            .or_else(|| ip.preferred())
            .and_then(|address| geolocate(&state.geoip, address, &mut lookup));
        claim.update(unix_now(), host, stats, geoip);
    };
    if let Some((code, reason)) = refusal {
        connection.close(code, reason).await;
    } else if let Some(claim) = claim {
        tracing::info!(node = %claim.name(), "agent disconnected");
    }
}

/// The country known for `ip` now. When it has yet to be found, starts `lookup` to fill it in
/// once found, unless that already looks `ip` up; a later report moving on drops it.
fn geolocate(geoip: &GeoIp, ip: IpAddr, lookup: &mut Option<Lookup>) -> Option<String> {
    if let Some(country) = geoip.known_country_code(ip) {
        *lookup = None;
        return country;
    }
    if lookup
        .as_ref()
        .is_none_or(|&(looking_up, _)| looking_up != ip)
    {
        let geoip = geoip.clone();
        *lookup = Some((
            ip,
            tokio::spawn(async move { geoip.country_code(ip).await }),
        ));
    }
    None
}

/// Waits for the country `lookup` finds, if there is one running, and clears it once done.
async fn looked_up(lookup: &mut Option<Lookup>) -> Option<String> {
    let Some((_, task)) = lookup else {
        return pending().await;
    };
    // A lookup that panicked found nothing.
    let country = task.await.ok().flatten();
    *lookup = None;
    country
}
