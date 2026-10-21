//! The channel the web dashboard polls.

use aiom_protocol::DASHBOARD_PING;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;

use super::Connection;
use crate::AppState;

/// The dashboard only ever sends pings.
const MAX_MESSAGE_SIZE: usize = 1024;

pub(crate) async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.max_message_size(MAX_MESSAGE_SIZE)
        .max_frame_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle(socket, state))
}

/// Answers every ping with the current node list.
async fn handle(socket: WebSocket, state: AppState) {
    let mut connection = Connection::new(socket, &state);
    while let Some(message) = connection.recv().await {
        if let Message::Text(text) = message
            && text.as_str() == DASHBOARD_PING
        {
            let Some(payload) = state.snapshots.get(&state.nodes) else {
                continue;
            };
            if !connection.send(Message::Binary(payload)).await {
                break;
            }
        }
    }
}
