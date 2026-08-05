//! Webhook-based messaging platforms — WhatsApp Cloud + Microsoft Graph
//! change-notification ingress (hermes `gateway/platforms/whatsapp_cloud.py`
//! and `gateway/platforms/msgraph_webhook.py` ports).
//!
//! Unlike the polling/websocket adapters in `messaging.rs`, these platforms
//! receive HTTP callbacks, so they mount as routes on the gateway router:
//!
//! * `GET  /webhooks/whatsapp`  — Meta verify handshake (hub.challenge echo)
//! * `POST /webhooks/whatsapp`  — inbound messages (HMAC-SHA256 verified)
//! * `GET/POST /webhooks/msgraph` — Graph validation + change notifications
//!
//! Outbound WhatsApp replies go through the Graph API messages endpoint;
//! both platforms share the messaging allowlist∪pairing union gate.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::error::{AgentError, Result};
use crate::messaging::{Dispatcher, MessageEvent};

/// Outbound chunk ceiling (hermes `_outgoing_chunk_limit`).
const WHATSAPP_CHUNK_LIMIT: usize = 4096;
/// Inbound webhook body cap (hermes `_read_limited_request_body`).
const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// WhatsApp Cloud (hermes whatsapp_cloud.py)
// ---------------------------------------------------------------------------

/// `[messaging.whatsapp_cloud]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WhatsAppCloudConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Graph API permanent access token (system user recommended).
    #[serde(default)]
    pub access_token: String,
    /// The business phone number id that receives messages.
    #[serde(default)]
    pub phone_number_id: String,
    /// Shared secret for Meta's `hub.verify_token` handshake.
    #[serde(default)]
    pub verify_token: String,
    /// App secret for `X-Hub-Signature-256` HMAC verification.
    #[serde(default)]
    pub app_secret: String,
    /// Sender phone numbers (wa_id) allowed to talk to the bot.
    /// Empty = pairing-only (fail closed until someone pairs).
    #[serde(default)]
    pub allowed_sender_ids: Vec<String>,
    /// Graph API version prefix.
    #[serde(default = "default_graph_version")]
    pub graph_version: String,
}

fn default_graph_version() -> String {
    "v20.0".to_string()
}

impl WhatsAppCloudConfig {
    fn graph_url(&self, path: &str) -> String {
        format!(
            "https://graph.facebook.com/{}/{path}",
            self.graph_version
        )
    }
}

/// Meta verify handshake (hermes `_handle_verify`): `hub.mode=subscribe` +
/// matching `hub.verify_token` → echo `hub.challenge`, else 403.
pub fn whatsapp_verify(cfg: &WhatsAppCloudConfig, query: &[(String, String)]) -> Result<String> {
    let get = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    if cfg.verify_token.is_empty() {
        return Err(AgentError::config(
            "whatsapp_cloud verify_token is not configured",
        ));
    }
    if get("hub.mode") == "subscribe" && get("hub.verify_token") == cfg.verify_token {
        Ok(get("hub.challenge"))
    } else {
        Err(AgentError::config("whatsapp verify handshake failed"))
    }
}

/// Verify `X-Hub-Signature-256` over the raw body (hermes
/// `_verify_signature`, constant-time). Missing app_secret disables the
/// check (hermes warns; the verify-token handshake still gates setup).
pub fn whatsapp_signature_ok(raw_body: &[u8], header: &str, app_secret: &str) -> bool {
    if app_secret.is_empty() {
        return true;
    }
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let Some(provided_hex) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(raw_body);
    let expected = mac.finalize().into_bytes();
    let provided = hex_bytes(provided_hex);
    provided.len() == expected.len() && constant_time_eq_bytes(&provided, expected.as_slice())
}

fn hex_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let (Some(a), Some(b)) = (chars.next(), chars.next()) {
        match u8::from_str_radix(&format!("{a}{b}"), 16) {
            Ok(byte) => out.push(byte),
            Err(_) => return Vec::new(),
        }
    }
    out
}

fn constant_time_eq_bytes(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Parse one webhook payload into text message events (hermes inbound
/// shape: entry[].changes[].value.messages[], type=text).
pub fn whatsapp_parse_messages(payload: &Value) -> Vec<MessageEvent> {
    let mut events = Vec::new();
    let Some(entries) = payload.get("entry").and_then(|v| v.as_array()) else {
        return events;
    };
    for entry in entries {
        let Some(changes) = entry.get("changes").and_then(|v| v.as_array()) else {
            continue;
        };
        for change in changes {
            let Some(messages) = change
                .pointer("/value/messages")
                .and_then(|v| v.as_array())
            else {
                continue;
            };
            for message in messages {
                if message.get("type").and_then(|v| v.as_str()) != Some("text") {
                    continue;
                }
                let from = message
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = message
                    .pointer("/text/body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if from.is_empty() || text.is_empty() {
                    continue;
                }
                events.push(MessageEvent {
                    platform: "whatsapp_cloud".into(),
                    chat_id: from.clone(),
                    sender_id: from,
                    sender_name: String::new(),
                    text,
                    message_id: message
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    attachments: Vec::new(),
                });
            }
        }
    }
    events
}

/// Send a text reply via the Graph API (hermes `send`): chunked, bearer
/// auth, `messaging_product: whatsapp`.
pub async fn whatsapp_send(
    client: &reqwest::Client,
    cfg: &WhatsAppCloudConfig,
    to: &str,
    text: &str,
) -> Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let url = cfg.graph_url("messages");
    for chunk in crate::messaging::chunk_text(text, WHATSAPP_CHUNK_LIMIT) {
        let payload = json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            "text": {"body": chunk, "preview_url": true},
        });
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", cfg.access_token))
            .json(&payload)
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("whatsapp send: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::Tool(format!(
                "whatsapp send failed ({status}): {body}"
            )));
        }
    }
    Ok(())
}

/// Full inbound handling: signature → parse → authz union (allowlist OR
/// pairing) → dispatch → reply. Returns Ok(true) when the payload was
/// accepted (200 to Meta either way, hermes ack semantics).
pub async fn whatsapp_handle_webhook(
    cfg: &WhatsAppCloudConfig,
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    raw_body: &[u8],
    signature_header: &str,
) -> Result<()> {
    if raw_body.len() > MAX_WEBHOOK_BODY_BYTES {
        return Err(AgentError::config("whatsapp webhook body too large"));
    }
    if !whatsapp_signature_ok(raw_body, signature_header, &cfg.app_secret) {
        return Err(AgentError::config(
            "whatsapp webhook rejected: invalid X-Hub-Signature-256",
        ));
    }
    let payload: Value = serde_json::from_slice(raw_body)
        .map_err(|e| AgentError::config(format!("whatsapp webhook parse: {e}")))?;
    let events = whatsapp_parse_messages(&payload);
    for mut event in events {
        let authorized = cfg
            .allowed_sender_ids
            .iter()
            .any(|allowed| *allowed == event.sender_id)
            || pairing
                .map(|store| store.is_approved("whatsapp_cloud", &event.sender_id))
                .unwrap_or(false);
        if !authorized {
            eprintln!(
                "[whatsapp_cloud] refusing message from {} — add it to \
                 messaging.whatsapp_cloud.allowed_sender_ids or approve a pairing code",
                event.sender_id
            );
            if let Some(store) = pairing {
                if let Some(reply) = crate::messaging::pairing_offer_public(
                    store,
                    "whatsapp_cloud",
                    &event.sender_id,
                    &event.sender_name,
                ) {
                    let client = reqwest::Client::new();
                    if let Err(e) = whatsapp_send(&client, cfg, &event.chat_id, &reply).await {
                        eprintln!("[whatsapp_cloud] pairing reply failed: {e}");
                    }
                }
            }
            continue;
        }
        // Pre-dispatch plugin gate (hermes ordering parity).
        if !crate::messaging::pre_gateway_dispatch_gate_public(&mut event).await {
            continue;
        }
        let reply = match dispatcher.handle_event(event.clone()).await {
            Ok(text) => text,
            Err(e) => format!("error: {e}"),
        };
        let (reply_text, media_paths) = crate::messaging::extract_media_tags(&reply);
        let client = reqwest::Client::new();
        if let Err(e) = whatsapp_send(&client, cfg, &event.chat_id, &reply_text).await {
            eprintln!("[whatsapp_cloud] reply failed: {e}");
        }
        if !media_paths.is_empty() {
            eprintln!(
                "[whatsapp_cloud] {} MEDIA attachment(s) not delivered (media upload not ported)",
                media_paths.len()
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Microsoft Graph change-notification ingress (hermes msgraph_webhook.py)
// ---------------------------------------------------------------------------

/// `[messaging.msgraph]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MsGraphConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Shared secret Graph echoes back on every notification — required
    /// (hermes refuses to start without it).
    #[serde(default)]
    pub client_state: String,
}

/// Graph subscription validation: echo `validationToken` as `text/plain`
/// (hermes `_handle_validation`).
pub fn msgraph_validation_token(query: &[(String, String)]) -> Option<String> {
    query
        .iter()
        .find(|(k, _)| k == "validationToken")
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

/// Verify one notification's clientState (hermes `_verify_client_state`).
pub fn msgraph_state_ok(notification: &Value, expected: &str) -> bool {
    notification
        .get("clientState")
        .and_then(|v| v.as_str())
        .map(|state| constant_time_eq_bytes(state.as_bytes(), expected.as_bytes()))
        .unwrap_or(false)
}

/// Handle a Graph notification batch: validate clientState per
/// notification and surface accepted ones as per-resource events.
/// Returns the number of accepted notifications.
pub async fn msgraph_handle_webhook(
    cfg: &MsGraphConfig,
    dispatcher: &Arc<Dispatcher>,
    raw_body: &[u8],
    query: &[(String, String)],
) -> Result<usize> {
    if raw_body.len() > MAX_WEBHOOK_BODY_BYTES {
        return Err(AgentError::config("msgraph webhook body too large"));
    }
    // Graph may tack a validationToken onto a POST right after subscription.
    if let Some(_token) = msgraph_validation_token(query) {
        return Ok(0);
    }
    if cfg.client_state.is_empty() {
        return Err(AgentError::config(
            "msgraph refuses notifications without client_state configured",
        ));
    }
    let payload: Value = serde_json::from_slice(raw_body)
        .map_err(|e| AgentError::config(format!("msgraph webhook parse: {e}")))?;
    let Some(notifications) = payload.get("value").and_then(|v| v.as_array()) else {
        return Ok(0);
    };
    let mut accepted = 0usize;
    for notification in notifications {
        if !msgraph_state_ok(notification, &cfg.client_state) {
            eprintln!("[msgraph] rejected notification: clientState mismatch");
            continue;
        }
        accepted += 1;
        let resource = notification
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let change_type = notification
            .get("changeType")
            .and_then(|v| v.as_str())
            .unwrap_or("changed")
            .to_string();
        // Hermes forwards the notification to a fetcher (Teams/Outlook)
        // that retrieves the actual message; ulnclaw surfaces the change
        // itself so workflows can react (documented difference).
        let event = MessageEvent {
            platform: "msgraph".into(),
            chat_id: resource.clone(),
            sender_id: "graph".into(),
            sender_name: "Microsoft Graph".into(),
            text: format!("[graph notification] {change_type}: {resource}"),
            message_id: notification
                .get("subscriptionId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            attachments: Vec::new(),
        };
        match dispatcher.handle_event(event).await {
            Ok(_) => {}
            Err(e) => eprintln!("[msgraph] dispatch failed: {e}"),
        }
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wa_cfg() -> WhatsAppCloudConfig {
        WhatsAppCloudConfig {
            enabled: true,
            access_token: "token".into(),
            phone_number_id: "123456".into(),
            verify_token: "seekrit".into(),
            app_secret: "app-secret".into(),
            allowed_sender_ids: vec!["5511999990000".into()],
            graph_version: "v20.0".into(),
        }
    }

    #[test]
    fn verify_handshake_echoes_challenge() {
        let cfg = wa_cfg();
        let query = vec![
            ("hub.mode".to_string(), "subscribe".to_string()),
            ("hub.verify_token".to_string(), "seekrit".to_string()),
            ("hub.challenge".to_string(), "1158201444".to_string()),
        ];
        assert_eq!(whatsapp_verify(&cfg, &query).unwrap(), "1158201444");

        let bad = vec![
            ("hub.mode".to_string(), "subscribe".to_string()),
            ("hub.verify_token".to_string(), "wrong".to_string()),
            ("hub.challenge".to_string(), "x".to_string()),
        ];
        assert!(whatsapp_verify(&cfg, &bad).is_err());
    }

    #[test]
    fn signature_verification_matches_meta_scheme() {
        let body = br#"{"object":"whatsapp_business_account"}"#;
        // Compute the expected signature with the same algorithm.
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"app-secret").unwrap();
        mac.update(body);
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(whatsapp_signature_ok(body, &format!("sha256={hex}"), "app-secret"));
        assert!(!whatsapp_signature_ok(body, "sha256=deadbeef", "app-secret"));
        assert!(!whatsapp_signature_ok(body, "", "app-secret"));
        // No app_secret configured → verification disabled (hermes).
        assert!(whatsapp_signature_ok(body, "", ""));
    }

    #[test]
    fn parses_text_messages_and_skips_others() {
        let payload = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "1",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messages": [
                            {"from": "5511999990000", "id": "wamid.1", "type": "text",
                             "text": {"body": "olá"}},
                            {"from": "5511999990000", "id": "wamid.2", "type": "image"},
                        ]
                    }
                }]
            }]
        });
        let events = whatsapp_parse_messages(&payload);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].platform, "whatsapp_cloud");
        assert_eq!(events[0].chat_id, "5511999990000");
        assert_eq!(events[0].text, "olá");
        assert_eq!(events[0].message_id, "wamid.1");
    }

    #[test]
    fn graph_url_uses_version_and_phone_id() {
        let cfg = wa_cfg();
        assert_eq!(
            cfg.graph_url("messages"),
            "https://graph.facebook.com/v20.0/messages"
        );
    }

    #[test]
    fn msgraph_validation_and_state() {
        let query = vec![("validationToken".to_string(), "abc123".to_string())];
        assert_eq!(msgraph_validation_token(&query).as_deref(), Some("abc123"));
        assert!(msgraph_validation_token(&[]).is_none());

        let notification = json!({"clientState": "s3cr3t", "resource": "teams/1"});
        assert!(msgraph_state_ok(&notification, "s3cr3t"));
        assert!(!msgraph_state_ok(&notification, "other"));
        assert!(!msgraph_state_ok(&json!({}), "s3cr3t"));
    }
}
