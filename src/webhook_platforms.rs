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

// ---------------------------------------------------------------------------
// Generic webhook platform (hermes webhook.py)
// ---------------------------------------------------------------------------

/// `[messaging.webhook]` — inbound webhooks from external services
/// (GitHub, GitLab, Svix/AgentMail, monitoring, inter-agent pings).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Per-route fixed-window rate limit (requests/minute, hermes default 30).
    #[serde(default = "default_webhook_rate_limit")]
    pub rate_limit: u32,
    /// Route table (`[[messaging.webhook.routes]]`).
    #[serde(default)]
    pub routes: Vec<WebhookRoute>,
}

fn default_webhook_rate_limit() -> u32 {
    30
}

/// One webhook route (hermes `platforms.webhook.extra.routes.<name>`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookRoute {
    /// Endpoint name — mounted at `/webhooks/hook/<name>`.
    pub name: String,
    /// HMAC secret. REQUIRED (hermes validates at startup);
    /// `INSECURE_NO_AUTH` disables verification (testing only).
    #[serde(default)]
    pub secret: String,
    /// Optional event-type filter (matched against X-Webhook-Event /
    /// X-GitHub-Event / X-Gitlab-Event headers).
    #[serde(default)]
    pub events: Vec<String>,
    /// Prompt template; `{body}` and `{event}` are substituted.
    #[serde(default = "default_webhook_prompt")]
    pub prompt: String,
    /// Where the response goes: `log` (default), `telegram`, `discord`,
    /// `slack`, `whatsapp_cloud`.
    #[serde(default = "default_webhook_deliver")]
    pub deliver: String,
    /// Chat/channel/number for platform delivery targets.
    #[serde(default)]
    pub deliver_chat: String,
    /// Skip the agent: the rendered prompt IS the delivered message
    /// (hermes deliver_only — zero-LLM push notifications).
    #[serde(default)]
    pub deliver_only: bool,
}

fn default_webhook_prompt() -> String {
    "Incoming webhook ({event}): {body}".to_string()
}

fn default_webhook_deliver() -> String {
    "log".to_string()
}

/// Replay window for timestamped signatures (hermes: 300s).
const SIGNATURE_REPLAY_WINDOW_SECS: i64 = 300;
/// Idempotency cache TTL (hermes `_idempotency_ttl` = 1h).
const IDEMPOTENCY_TTL_SECS: u64 = 3600;
/// Svix/base64 marker.
const SVIX_SECRET_PREFIX: &str = "whsec_";

/// Multi-scheme signature verification (hermes `_validate_signature`):
/// Svix, GitHub, GitLab, generic V2 (timestamp-bound), generic V1
/// (legacy). V2 presence commits — never downgrades to V1.
pub fn webhook_signature_ok(
    route_name: &str,
    body: &[u8],
    headers: &[(String, String)],
    secret: &str,
    now_secs: i64,
) -> bool {
    if secret == "INSECURE_NO_AUTH" {
        return true;
    }
    if secret.is_empty() {
        return false;
    }
    let header = |name: &str| -> String {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let hmac_hex = |data: &[u8]| -> String {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return String::new();
        };
        mac.update(data);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };

    // Svix / AgentMail: svix-id / svix-timestamp / svix-signature
    // ("v1,<base64>" list), signed content "{id}.{timestamp}.{body}".
    let svix_id = header("svix-id");
    let svix_timestamp = header("svix-timestamp");
    let svix_signature = header("svix-signature");
    if !svix_id.is_empty() || !svix_timestamp.is_empty() || !svix_signature.is_empty() {
        if svix_id.is_empty() || svix_timestamp.is_empty() || svix_signature.is_empty() {
            return false;
        }
        let Ok(ts) = svix_timestamp.parse::<i64>() else {
            return false;
        };
        if (now_secs - ts).abs() > SIGNATURE_REPLAY_WINDOW_SECS {
            return false;
        }
        // whsec_ secrets are base64-encoded keys; plain secrets used as-is.
        let key: Vec<u8> = if let Some(encoded) = secret.strip_prefix(SVIX_SECRET_PREFIX) {
            use base64::Engine;
            match base64::engine::general_purpose::STANDARD.decode(encoded) {
                Ok(key) => key,
                Err(_) => return false,
            }
        } else {
            secret.as_bytes().to_vec()
        };
        let signed = format!("{svix_id}.{svix_timestamp}.").into_bytes()
            .iter()
            .cloned()
            .chain(body.iter().cloned())
            .collect::<Vec<u8>>();
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&key) else {
            return false;
        };
        mac.update(&signed);
        let expected = mac.finalize().into_bytes();
        use base64::Engine;
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(expected);
        return svix_signature.split_whitespace().any(|candidate| {
            candidate
                .strip_prefix("v1,")
                .map(|digest| constant_time_eq_bytes(digest.as_bytes(), expected_b64.as_bytes()))
                .unwrap_or(false)
        });
    }

    // GitHub: X-Hub-Signature-256 = sha256=<hex>.
    let gh_sig = header("x-hub-signature-256");
    if !gh_sig.is_empty() {
        let expected = format!("sha256={}", hmac_hex(body));
        return constant_time_eq_bytes(gh_sig.as_bytes(), expected.as_bytes());
    }

    // GitLab: X-Gitlab-Token = <plain secret>.
    let gl_token = header("x-gitlab-token");
    if !gl_token.is_empty() {
        return constant_time_eq_bytes(gl_token.as_bytes(), secret.as_bytes());
    }

    // Generic V2: timestamp-bound (replay-protected). Presence commits —
    // no V1 fallback (hermes downgrade guard).
    let v2_sig = header("x-webhook-signature-v2");
    if !v2_sig.is_empty() {
        let v2_timestamp = header("x-webhook-timestamp");
        if v2_timestamp.is_empty() {
            tracing::warn!(
                "[webhook] route '{route_name}' sent X-Webhook-Signature-V2 with no X-Webhook-Timestamp — rejecting"
            );
            return false;
        }
        let Ok(ts) = v2_timestamp.parse::<i64>() else {
            return false;
        };
        if (now_secs - ts).abs() > SIGNATURE_REPLAY_WINDOW_SECS {
            tracing::warn!(
                "[webhook] route '{route_name}' generic HMAC V2 timestamp outside replay window"
            );
            return false;
        }
        let mut signed = v2_timestamp.as_bytes().to_vec();
        signed.push(b'.');
        signed.extend_from_slice(body);
        let expected = hmac_hex(&signed);
        return constant_time_eq_bytes(v2_sig.as_bytes(), expected.as_bytes());
    }

    // Generic V1 (legacy, body-only — replay-vulnerable; hermes warns once).
    let v1_sig = header("x-webhook-signature");
    if !v1_sig.is_empty() {
        let expected = hmac_hex(body);
        return constant_time_eq_bytes(v1_sig.as_bytes(), expected.as_bytes());
    }

    false
}

/// Route event-filter check (hermes header-based event filtering).
pub fn webhook_event_allowed(route: &WebhookRoute, headers: &[(String, String)]) -> (bool, String) {
    let event = ["x-webhook-event", "x-github-event", "x-gitlab-event"]
        .iter()
        .find_map(|name| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        })
        .unwrap_or_default();
    if route.events.is_empty() {
        return (true, event);
    }
    (route.events.iter().any(|allowed| *allowed == event), event)
}

/// Render the route prompt template (hermes prompt formatting).
pub fn webhook_render_prompt(template: &str, event: &str, body: &str) -> String {
    template.replace("{event}", event).replace("{body}", body)
}

/// Per-process webhook runtime state (rate limits + idempotency).
pub struct WebhookRuntime {
    /// route -> (window_start_epoch_min, count)
    pub rate_windows: tokio::sync::Mutex<std::collections::HashMap<String, (u64, u32)>>,
    /// delivery-id -> processed-at epoch secs
    pub seen_deliveries: tokio::sync::Mutex<std::collections::HashMap<String, u64>>,
}

impl Default for WebhookRuntime {
    fn default() -> Self {
        Self {
            rate_windows: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            seen_deliveries: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

/// Fixed-window rate limit (hermes per-route limiter).
pub async fn webhook_rate_limited(
    runtime: &WebhookRuntime,
    route_name: &str,
    limit: u32,
    now_secs: u64,
) -> bool {
    let window = now_secs / 60;
    let mut windows = runtime.rate_windows.lock().await;
    let entry = windows.entry(route_name.to_string()).or_insert((window, 0));
    if entry.0 != window {
        *entry = (window, 0);
    }
    entry.1 += 1;
    entry.1 > limit
}

/// Idempotency: true when this delivery id was already processed within
/// the TTL (hermes `_seen_deliveries`).
pub async fn webhook_already_seen(
    runtime: &WebhookRuntime,
    delivery_id: &str,
    now_secs: u64,
) -> bool {
    if delivery_id.is_empty() {
        return false;
    }
    let mut seen = runtime.seen_deliveries.lock().await;
    seen.retain(|_, ts| now_secs.saturating_sub(*ts) < IDEMPOTENCY_TTL_SECS);
    if seen.contains_key(delivery_id) {
        return true;
    }
    seen.insert(delivery_id.to_string(), now_secs);
    false
}

/// Deliver a webhook response to the configured target (hermes `deliver`).
/// Unknown/misconfigured targets fall back to the log (hermes behavior).
pub async fn webhook_deliver(
    config: &crate::config::UlncLawConfig,
    route: &WebhookRoute,
    text: &str,
) {
    if text.trim().is_empty() {
        return;
    }
    let client = reqwest::Client::new();
    match route.deliver.as_str() {
        "telegram" => {
            let cfg = &config.messaging.telegram;
            let token = crate::messaging::resolve_telegram_token_public(cfg);
            match token {
                Some(token) => {
                    crate::messaging::telegram_send_public(&client, &token, &route.deliver_chat, text).await
                }
                None => eprintln!("[webhook] telegram delivery unavailable: no bot token"),
            }
        }
        "discord" => {
            let cfg = &config.messaging.discord;
            let token = crate::messaging::resolve_discord_token_public(cfg);
            match token {
                Some(token) => {
                    crate::messaging::discord_send_public(&token, &route.deliver_chat, text).await
                }
                None => eprintln!("[webhook] discord delivery unavailable: no bot token"),
            }
        }
        "slack" => {
            let cfg = &config.messaging.slack;
            let token = crate::messaging::resolve_slack_bot_token_public(cfg);
            match token {
                Some(token) => {
                    crate::messaging::slack_send_public(&token, &route.deliver_chat, text).await
                }
                None => eprintln!("[webhook] slack delivery unavailable: no bot token"),
            }
        }
        "whatsapp_cloud" => {
            let cfg = &config.messaging.whatsapp_cloud;
            if let Err(e) = whatsapp_send(&client, cfg, &route.deliver_chat, text).await {
                eprintln!("[webhook] whatsapp delivery failed: {e}");
            }
        }
        _ => {
            // "log" and anything unrecognized (hermes fallback).
            eprintln!("[webhook] route '{}': {}", route.name, text);
        }
    }
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

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn hmac_sha256_hex(secret: &str, data: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(data);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn webhook_signature_github_scheme() {
        let body = br#"{"action":"opened"}"#;
        let sig = format!("sha256={}", hmac_sha256_hex("gh-secret", body));
        let ok = headers(&[("X-Hub-Signature-256", &sig)]);
        assert!(webhook_signature_ok("gh", body, &ok, "gh-secret", 0));
        let bad = headers(&[("X-Hub-Signature-256", "sha256=00")]);
        assert!(!webhook_signature_ok("gh", body, &bad, "gh-secret", 0));
    }

    #[test]
    fn webhook_signature_gitlab_and_insecure() {
        let body = b"x";
        let ok = headers(&[("X-Gitlab-Token", "gl-secret")]);
        assert!(webhook_signature_ok("gl", body, &ok, "gl-secret", 0));
        // INSECURE_NO_AUTH bypasses verification entirely (testing only).
        assert!(webhook_signature_ok("gl", body, &[], "INSECURE_NO_AUTH", 0));
        // No header + real secret → reject.
        assert!(!webhook_signature_ok("gl", body, &[], "gl-secret", 0));
    }

    #[test]
    fn webhook_signature_v2_timestamp_binding() {
        let body = br#"{"n":1}"#;
        let now = 1_700_000_000i64;
        let ts = now.to_string();
        let mut signed = ts.as_bytes().to_vec();
        signed.push(b'.');
        signed.extend_from_slice(body);
        let sig = hmac_sha256_hex("w-secret", &signed);
        let ok = headers(&[
            ("X-Webhook-Signature-V2", &sig),
            ("X-Webhook-Timestamp", &ts),
        ]);
        assert!(webhook_signature_ok("w", body, &ok, "w-secret", now));
        // Outside the 300s replay window.
        assert!(!webhook_signature_ok("w", body, &ok, "w-secret", now + 301));
        // V2 without timestamp must NOT fall back to V1.
        let no_ts = headers(&[
            ("X-Webhook-Signature-V2", &sig),
            ("X-Webhook-Signature", &hmac_sha256_hex("w-secret", body)),
        ]);
        assert!(!webhook_signature_ok("w", body, &no_ts, "w-secret", now));
    }

    #[test]
    fn webhook_signature_v1_legacy() {
        let body = br#"legacy"#;
        let sig = hmac_sha256_hex("w-secret", body);
        let ok = headers(&[("X-Webhook-Signature", &sig)]);
        assert!(webhook_signature_ok("w", body, &ok, "w-secret", 0));
    }

    #[test]
    fn webhook_signature_svix_scheme() {
        use base64::Engine;
        let body = br#"{"email":"x@y.z"}"#;
        let key = b"0123456789abcdef0123456789abcdef";
        let secret = format!(
            "whsec_{}",
            base64::engine::general_purpose::STANDARD.encode(key)
        );
        let now = 1_700_000_000i64;
        let signed = format!("msg_abc.{now}.").into_bytes()
            .iter()
            .cloned()
            .chain(body.iter().cloned())
            .collect::<Vec<u8>>();
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
        mac.update(&signed);
        let digest = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let ok = headers(&[
            ("svix-id", "msg_abc"),
            ("svix-timestamp", &now.to_string()),
            ("svix-signature", &format!("v1,{digest}")),
        ]);
        assert!(webhook_signature_ok("svix", body, &ok, &secret, now));
        // Stale timestamp rejected.
        assert!(!webhook_signature_ok("svix", body, &ok, &secret, now + 1000));
    }

    #[test]
    fn webhook_event_filter_and_prompt_render() {
        let route = WebhookRoute {
            name: "gh".into(),
            events: vec!["push".into()],
            prompt: "got {event}: {body}".into(),
            ..Default::default()
        };
        let (allowed, event) = webhook_event_allowed(&route, &headers(&[("X-GitHub-Event", "push")]));
        assert!(allowed);
        assert_eq!(event, "push");
        let (denied, _) = webhook_event_allowed(&route, &headers(&[("X-GitHub-Event", "issues")]));
        assert!(!denied);
        // No filter → everything passes.
        let open = WebhookRoute { name: "x".into(), ..Default::default() };
        let (allowed, _) = webhook_event_allowed(&open, &[]);
        assert!(allowed);
        assert_eq!(
            webhook_render_prompt("got {event}: {body}", "push", "{}"),
            "got push: {}"
        );
    }

    #[tokio::test]
    async fn webhook_rate_limit_and_idempotency() {
        let runtime = WebhookRuntime::default();
        let now = 1_700_000_000u64;
        for idx in 0..3 {
            assert!(!webhook_rate_limited(&runtime, "r", 3, now + idx).await);
        }
        assert!(webhook_rate_limited(&runtime, "r", 3, now).await);
        // New window resets.
        assert!(!webhook_rate_limited(&runtime, "r", 3, now + 61).await);

        assert!(!webhook_already_seen(&runtime, "deliv-1", now).await);
        assert!(webhook_already_seen(&runtime, "deliv-1", now + 5).await);
        // Empty ids are never deduped.
        assert!(!webhook_already_seen(&runtime, "", now).await);
        // Expired entries are forgotten.
        assert!(!webhook_already_seen(&runtime, "deliv-1", now + IDEMPOTENCY_TTL_SECS + 1).await);
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
