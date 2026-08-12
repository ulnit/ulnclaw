//! `/api/ws` — dashboard live-event WebSocket (hermes desktop parity).
//!
//! The hermes desktop shell validates every resolved backend by opening
//! `ws://host:port/api/ws?token=…` and keeps it as the renderer's live
//! event door. This bridge accepts the upgrade (token-gated exactly like
//! the HTTP surface), greets with a `hello` frame, then forwards the
//! desktop event bus as JSON envelopes plus a 15s keepalive ping.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

use crate::gateway::GatewayState;

/// `GET /api/ws` — upgrade to the live event socket.
pub async fn dashboard_ws(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<GatewayState>>,
) -> Response {
    let token = params.get("token").cloned();
    if let Some(key) = state.key.as_ref() {
        if token.as_deref() != Some(key.as_str()) {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response();
        }
    }
    ws.on_upgrade(serve_socket)
}

async fn serve_socket(mut socket: WebSocket) {
    let hello = json!({
        "type": "hello",
        "server": "ulnclaw",
        "version": env!("CARGO_PKG_VERSION"),
    });
    if socket.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    let mut bus = crate::desktop_bridge::subscribe();
    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(15));
    keepalive.tick().await; // first tick is immediate; drop it
    loop {
        tokio::select! {
            event = bus.recv() => {
                let Ok(event) = event else { return };
                let frame = json!({
                    "type": "event",
                    "event": event.event,
                    "session_id": event.session_id,
                    "payload": event.payload,
                });
                if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                    return;
                }
            }
            _ = keepalive.tick() => {
                if socket.send(Message::Text("{\"type\":\"ping\"}".into())).await.is_err() {
                    return;
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() { return; }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => return,
                }
            }
        }
    }
}
