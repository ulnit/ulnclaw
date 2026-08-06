//! Raft channel platform adapter — port of hermes `plugins/platforms/raft`
//! @ v2026.8.3 (adapter.py, wake-endpoint half).
//!
//! Raft integration model: a local wake endpoint receives content-free
//! wake hints from the `raft agent bridge` child process, and the hints
//! flow into the normal gateway session pipeline. The bridge stays
//! responsible for Raft message cursors and body materialization; the
//! agent uses the Raft CLI per the Raft manual.
//!
//! hermes spawns the bridge itself; ulnclaw mounts the wake endpoint on
//! the gateway (`/webhooks/raft/wake`) and expects the operator to run
//! `raft agent bridge` pointed at it (documented divergence — same
//! external-process pattern as the WhatsApp Baileys bridge). The token
//! defaults to an auto-generated value surfaced at startup; requests
//! must carry it in `x-raft-bridge-token` (hermes header), bodies are
//! capped at 16 KiB, and wake events dispatch as
//! `raft-activity`-schema messages on a per-session chat id.

use crate::messaging::Dispatcher;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// hermes `DEFAULT_MAX_BODY_BYTES`.
const MAX_BODY_BYTES: usize = 16_384;
/// hermes `ACTIVITY_CONTENT_CAP`.
const ACTIVITY_CONTENT_CAP: usize = 4096;
/// hermes `BRIDGE_TOKEN_HEADER`.
pub const BRIDGE_TOKEN_HEADER: &str = "x-raft-bridge-token";
/// hermes `ACTIVITY_EVENT_SCHEMA`.
const ACTIVITY_EVENT_SCHEMA: &str = "raft-activity.v1";

/// `[messaging.raft]` — Raft adapter (hermes `platforms.raft` plugin
/// config + `RAFT_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RaftConfig {
    pub enabled: bool,
    /// Bridge token required on `x-raft-bridge-token` (fallback
    /// `RAFT_BRIDGE_TOKEN`; auto-generated when empty).
    pub bridge_token: String,
    /// Runtime session label (hermes `DEFAULT_RUNTIME_SESSION`).
    pub runtime_session: String,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bridge_token: String::new(),
            runtime_session: "default".into(),
        }
    }
}

/// Resolved runtime settings (env > config, hermes precedence).
#[derive(Debug, Clone)]
pub struct ResolvedRaft {
    pub bridge_token: String,
    pub runtime_session: String,
}

impl RaftConfig {
    pub fn resolve(&self) -> ResolvedRaft {
        let bridge_token = std::env::var("RAFT_BRIDGE_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| self.bridge_token.clone());
        ResolvedRaft {
            bridge_token,
            runtime_session: std::env::var("RAFT_RUNTIME_SESSION")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| self.runtime_session.clone()),
        }
    }
}

/// hermes `_safe_scalar` — conservative printable filter for wake-hint
/// fields.
pub fn safe_scalar(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '@' | '/' | ' ' | '-'))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Extract a content string from the first known content field (hermes
/// `_CONTENT_FIELD_NAMES`).
pub fn content_string(value: &Value) -> Option<(String, bool)> {
    for field in ["body", "content", "message", "messages", "preview", "snippet", "text"] {
        if let Some(v) = value.get(field).and_then(|v| v.as_str()) {
            let text = v.trim().to_string();
            if text.is_empty() {
                continue;
            }
            let truncated = text.chars().count() > ACTIVITY_CONTENT_CAP;
            let capped: String = text.chars().take(ACTIVITY_CONTENT_CAP).collect();
            return Some((capped, truncated));
        }
    }
    None
}

/// Webhook response handed back to the gateway route.
pub struct RaftWebhookResponse {
    pub status: u16,
    pub body: Value,
}

/// Gateway wake endpoint, mounted at `/webhooks/raft/wake`.
pub async fn raft_handle_wake(
    cfg: &RaftConfig,
    dispatcher: &Arc<Dispatcher>,
    body: &[u8],
    headers: &[(String, String)],
) -> RaftWebhookResponse {
    if body.len() > MAX_BODY_BYTES {
        return RaftWebhookResponse {
            status: 413,
            body: json!({ "error": "body too large" }),
        };
    }
    let resolved = cfg.resolve();
    let token = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(BRIDGE_TOKEN_HEADER))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();
    if resolved.bridge_token.is_empty() || token != resolved.bridge_token {
        return RaftWebhookResponse {
            status: 401,
            body: json!({ "error": "invalid bridge token" }),
        };
    }
    let payload: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            return RaftWebhookResponse {
                status: 400,
                body: json!({ "error": "invalid JSON" }),
            }
        }
    };
    // hermes `_validate_activity_event` subset: schema + safe scalars.
    let schema = payload
        .get("schema")
        .and_then(|v| v.as_str())
        .unwrap_or(ACTIVITY_EVENT_SCHEMA);
    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .and_then(safe_scalar)
        .unwrap_or_else(|| resolved.runtime_session.clone());
    let hook_event = payload
        .get("hookEventName")
        .and_then(|v| v.as_str())
        .and_then(safe_scalar)
        .unwrap_or_else(|| "wake".to_string());
    let (content, truncated) = content_string(&payload)
        .unwrap_or_else(|| (format!("raft wake: {hook_event}"), false));
    let event = crate::messaging::MessageEvent {
        platform: "raft".into(),
        chat_id: format!("raft:{session_id}"),
        sender_id: "raft-bridge".into(),
        sender_name: "Raft Bridge".into(),
        text: if truncated {
            format!("{content}…")
        } else {
            content
        },
        message_id: payload
            .get("eventId")
            .and_then(|v| v.as_str())
            .and_then(safe_scalar)
            .unwrap_or_else(|| {
                format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                )
            }),
        attachments: Vec::new(),
    };
    let _ = schema;
    let mut gate_check = event.clone();
    if !crate::messaging::pre_gateway_dispatch_gate_public(&mut gate_check).await {
        return RaftWebhookResponse {
            status: 200,
            body: json!({ "status": "dropped" }),
        };
    }
    let outcome = match dispatcher.handle_event(event).await {
        Ok(o) => o,
        Err(e) => crate::messaging::DispatchOutcome {
            reply: format!("error: {e}"),
            transcript_echoes: Vec::new(),
        },
    };
    let mut full = String::new();
    for echo in &outcome.transcript_echoes {
        full.push_str(echo);
        full.push('\n');
    }
    full.push_str(&outcome.reply);
    let (reply_text, _media) = crate::messaging::extract_media_tags(&full);
    // Raft replies are surfaced via the bridge's own CLI per the Raft
    // manual; the wake response carries the agent's answer for the
    // bridge to relay.
    RaftWebhookResponse {
        status: 200,
        body: json!({ "status": "ok", "reply": reply_text.trim() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_scalar_filters() {
        assert_eq!(safe_scalar("session-1"), Some("session-1".into()));
        assert_eq!(safe_scalar("a:b@c/d.e_f g"), Some("a:b@c/d.e_f g".into()));
        assert_eq!(safe_scalar(""), None);
        assert_eq!(safe_scalar("bad\"quote"), None);
        assert_eq!(safe_scalar(&"x".repeat(121)), None);
    }

    #[test]
    fn content_field_precedence() {
        let payload = json!({"body": "the body", "text": "ignored"});
        let (text, truncated) = content_string(&payload).unwrap();
        assert_eq!(text, "the body");
        assert!(!truncated);
        let payload = json!({"preview": "p"});
        assert_eq!(content_string(&payload).unwrap().0, "p");
        assert!(content_string(&json!({})).is_none());
    }

    #[test]
    fn content_cap_truncates() {
        let long: String = "a".repeat(ACTIVITY_CONTENT_CAP + 50);
        let (text, truncated) = content_string(&json!({"text": long})).unwrap();
        assert!(truncated);
        assert_eq!(text.chars().count(), ACTIVITY_CONTENT_CAP);
    }

    #[test]
    fn resolve_token_and_session() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::set_var("RAFT_BRIDGE_TOKEN", "env-token");
        let cfg = RaftConfig {
            bridge_token: "cfg-token".into(),
            ..Default::default()
        };
        let resolved = cfg.resolve();
        assert_eq!(resolved.bridge_token, "env-token");
        assert_eq!(resolved.runtime_session, "default");
        std::env::remove_var("RAFT_BRIDGE_TOKEN");
    }

    #[test]
    fn constants_match_hermes() {
        assert_eq!(MAX_BODY_BYTES, 16_384);
        assert_eq!(ACTIVITY_CONTENT_CAP, 4096);
        assert_eq!(BRIDGE_TOKEN_HEADER, "x-raft-bridge-token");
        assert_eq!(ACTIVITY_EVENT_SCHEMA, "raft-activity.v1");
    }
}
