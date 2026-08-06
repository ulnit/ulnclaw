//! Google Chat platform adapter — port of hermes
//! `plugins/platforms/google_chat` @ v2026.8.3 (adapter.py, HTTP
//! callback transport).
//!
//! Inbound events arrive at the gateway-mounted `/webhooks/googlechat`
//! route (hermes `GOOGLE_CHAT_HTTP_EVENTS_URL` mode). Requests carry a
//! Google-issued ID token in `Authorization: Bearer`; the token is
//! verified via Google's `tokeninfo` endpoint and must match the
//! configured audience + the bot's service-account email(s) — hermes'
//! local-cert verification path is replaced by the online tokeninfo
//! probe (documented divergence; the Pub/Sub streaming-pull inbound
//! mode needs gRPC and is not ported).
//!
//! Envelope handling mirrors hermes: only `MESSAGE` events dispatch,
//! `BOT` senders are skipped, message names dedup (300 s window),
//! `argumentText` wins over `text`, DIRECT_MESSAGE spaces map to DMs,
//! and thread names are carried so replies stay in-thread.
//!
//! Outbound rides the Chat REST API with service-account credentials:
//! an RS256-signed JWT assertion (`jsonwebtoken`) exchanged at the
//! key's `token_uri` for an access token (cached until ~5 min before
//! expiry), then `POST /v1/{space}/messages` with 4000-char chunks
//! (hermes `_MAX_TEXT_LENGTH`). The per-user OAuth `media.upload`
//! flow (`oauth.py`) and typing-card patching are not ported.

use crate::messaging::{Dispatcher, MessageEvent};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// hermes `_MAX_TEXT_LENGTH` for Chat messages.
const MAX_MESSAGE_LENGTH: usize = 4000;
const DEDUP_WINDOW_SECS: u64 = 300;
const DEDUP_MAX_SIZE: usize = 1000;
const API_TIMEOUT: Duration = Duration::from_secs(30);
const CHAT_SCOPE: &str = "https://www.googleapis.com/auth/chat.bot";
const TOKENINFO_URL: &str = "https://oauth2.googleapis.com/tokeninfo";
/// Pub/Sub pull transport (hermes streaming_pull replacement — REST
/// pull speaks the same subscription without gRPC).
const PUBSUB_SCOPE: &str = "https://www.googleapis.com/auth/pubsub";
const PUBSUB_API_BASE: &str = "https://pubsub.googleapis.com/v1";
/// hermes `_MAX_RECONNECT_ATTEMPTS` / `_RECONNECT_BASE_DELAY` /
/// `_RECONNECT_MAX_DELAY` (full-jitter exponential backoff).
const MAX_RECONNECT_ATTEMPTS: u32 = 10;
const RECONNECT_BASE_DELAY_MS: u64 = 2_000;
const RECONNECT_MAX_DELAY_MS: u64 = 120_000;
/// hermes `GOOGLE_CHAT_MAX_MESSAGES` default.
const PULL_MAX_MESSAGES: u64 = 1;
/// Idle pacing between empty REST pulls (streaming pull blocks; REST
/// must not hot-spin).
const PULL_IDLE_DELAY: Duration = Duration::from_secs(1);

/// `[messaging.google_chat]` — Google Chat adapter (hermes
/// `platforms.google_chat` plugin config + `GOOGLE_CHAT_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleChatConfig {
    pub enabled: bool,
    /// Path to the service-account JSON key (fallback
    /// `GOOGLE_CHAT_SERVICE_ACCOUNT_FILE`).
    pub service_account_file: String,
    /// Expected audience for inbound ID tokens (fallback
    /// `GOOGLE_CHAT_HTTP_EVENTS_AUDIENCE`) — usually the webhook URL.
    pub http_events_audience: String,
    /// Comma list of service-account emails allowed to push events
    /// (fallback `GOOGLE_CHAT_HTTP_EVENTS_SERVICE_ACCOUNT`).
    pub http_events_service_account: String,
    /// Sender emails or user ids allowed to talk to the bot (fallback
    /// `GOOGLE_CHAT_ALLOWED_USERS`).
    pub allowed_users: Vec<String>,
    /// Cron/notification delivery space (fallback
    /// `GOOGLE_CHAT_HOME_CHANNEL`).
    pub home_channel: String,
    /// Pub/Sub subscription (`projects/<p>/subscriptions/<s>`) for the
    /// pull transport (fallback `GOOGLE_CHAT_PUBSUB_SUBSCRIPTION`).
    pub pubsub_subscription: String,
}

impl Default for GoogleChatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_account_file: String::new(),
            http_events_audience: String::new(),
            http_events_service_account: String::new(),
            allowed_users: Vec::new(),
            home_channel: String::new(),
            pubsub_subscription: String::new(),
        }
    }
}

fn env_trim(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_list(name: &str) -> Option<Vec<String>> {
    env_trim(name).map(|raw| {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// Resolved runtime settings (env > config, hermes precedence).
#[derive(Debug, Clone)]
pub struct ResolvedGoogleChat {
    pub service_account_file: String,
    pub http_events_audience: String,
    pub http_events_service_account: String,
    pub allowed_users: Vec<String>,
    pub home_channel: String,
    pub pubsub_subscription: String,
}

impl GoogleChatConfig {
    pub fn resolve(&self) -> ResolvedGoogleChat {
        ResolvedGoogleChat {
            service_account_file: env_trim("GOOGLE_CHAT_SERVICE_ACCOUNT_FILE")
                .unwrap_or_else(|| self.service_account_file.clone()),
            http_events_audience: env_trim("GOOGLE_CHAT_HTTP_EVENTS_AUDIENCE")
                .unwrap_or_else(|| self.http_events_audience.clone()),
            http_events_service_account: env_trim("GOOGLE_CHAT_HTTP_EVENTS_SERVICE_ACCOUNT")
                .unwrap_or_else(|| self.http_events_service_account.clone()),
            allowed_users: env_list("GOOGLE_CHAT_ALLOWED_USERS")
                .unwrap_or_else(|| self.allowed_users.clone()),
            home_channel: env_trim("GOOGLE_CHAT_HOME_CHANNEL")
                .unwrap_or_else(|| self.home_channel.clone()),
            pubsub_subscription: env_trim("GOOGLE_CHAT_PUBSUB_SUBSCRIPTION")
                .unwrap_or_else(|| self.pubsub_subscription.trim().to_string()),
        }
    }
}

/// Parsed service-account key (the fields the JWT flow needs).
#[derive(Debug, Clone)]
pub struct ServiceAccountKey {
    pub client_email: String,
    pub token_uri: String,
    pub private_key_pem: String,
}

/// Parse a Google service-account JSON key file.
pub fn parse_service_account_key(json_str: &str) -> Result<ServiceAccountKey, String> {
    let value: Value =
        serde_json::from_str(json_str).map_err(|e| format!("invalid key JSON: {e}"))?;
    let client_email = value
        .get("client_email")
        .and_then(|v| v.as_str())
        .ok_or("key missing client_email")?
        .to_string();
    let token_uri = value
        .get("token_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("https://oauth2.googleapis.com/token")
        .to_string();
    let private_key_pem = value
        .get("private_key")
        .and_then(|v| v.as_str())
        .ok_or("key missing private_key")?
        .to_string();
    Ok(ServiceAccountKey {
        client_email,
        token_uri,
        private_key_pem,
    })
}

/// Build the RS256-signed JWT assertion for the service-account flow.
pub fn build_jwt_assertion(key: &ServiceAccountKey) -> Result<String, String> {
    build_jwt_assertion_scoped(key, CHAT_SCOPE)
}

/// Scope-parameterized variant (the Pub/Sub transport claims the
/// `pubsub` scope).
pub fn build_jwt_assertion_scoped(key: &ServiceAccountKey, scope: &str) -> Result<String, String> {
    use jsonwebtoken::{EncodingKey, Header};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let claims = json!({
        "iss": key.client_email,
        "scope": scope,
        "aud": key.token_uri,
        "iat": now,
        "exp": now + 3600,
    });
    let encoding = EncodingKey::from_rsa_pem(key.private_key_pem.as_bytes())
        .map_err(|e| format!("private key parse: {e}"))?;
    jsonwebtoken::encode(&Header::new(jsonwebtoken::Algorithm::RS256), &claims, &encoding)
        .map_err(|e| format!("jwt encode: {e}"))
}

struct Runtime {
    cfg: ResolvedGoogleChat,
    client: reqwest::Client,
    key: Option<ServiceAccountKey>,
    /// Cached access token + expiry.
    token: Mutex<Option<(String, Instant)>>,
    /// Pub/Sub-scoped token slot.
    pubsub_token: Mutex<Option<(String, Instant)>>,
    /// Message-name dedup (hermes MessageDeduplicator).
    dedup: Mutex<HashMap<String, u64>>,
    /// chat_id -> thread name for in-thread replies.
    threads: Mutex<HashMap<String, String>>,
}

impl Runtime {
    async fn is_duplicate(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut dedup = self.dedup.lock().await;
        dedup.retain(|_, ts| now.saturating_sub(*ts) < DEDUP_WINDOW_SECS);
        if dedup.contains_key(name) {
            return true;
        }
        if dedup.len() >= DEDUP_MAX_SIZE {
            let mut entries: Vec<(String, u64)> = dedup.drain().collect();
            entries.sort_by_key(|(_, ts)| *ts);
            entries.truncate(DEDUP_MAX_SIZE / 2);
            *dedup = entries.into_iter().collect();
        }
        dedup.insert(name.to_string(), now);
        false
    }

    /// Service-account access token with cache.
    async fn access_token(&self) -> Result<String, String> {
        {
            let cached = self.token.lock().await;
            if let Some((token, expiry)) = cached.as_ref() {
                if Instant::now() < *expiry {
                    return Ok(token.clone());
                }
            }
        }
        let key = self
            .key
            .as_ref()
            .ok_or("no service-account key configured")?;
        let assertion = build_jwt_assertion(key)?;
        let resp = self
            .client
            .post(&key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("token request: {e}"))?;
        let status = resp.status();
        let payload: Value = resp.json().await.unwrap_or(json!({}));
        if status.as_u16() >= 400 {
            return Err(format!("token request failed ({status}): {payload}"));
        }
        let token = payload
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or("token response missing access_token")?
            .to_string();
        let expires_in = payload
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);
        *self.token.lock().await = Some((
            token.clone(),
            Instant::now() + Duration::from_secs(expires_in.saturating_sub(300)),
        ));
        Ok(token)
    }

    /// hermes `_create_message` — POST /v1/{space}/messages.
    async fn create_message(&self, space_name: &str, text: &str, thread_name: Option<&str>) -> Result<(), String> {
        let token = self.access_token().await?;
        let url = format!("https://chat.googleapis.com/v1/{space_name}/messages");
        let mut body = json!({ "text": text });
        if let Some(thread) = thread_name {
            body["thread"] = json!({ "name": thread });
        }
        let resp = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("message create: {e}"))?;
        if resp.status().as_u16() >= 400 {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("message create failed ({status}): {}", &err[..err.len().min(300)]));
        }
        Ok(())
    }
}

static RUNTIME: std::sync::OnceLock<Arc<Runtime>> = std::sync::OnceLock::new();

/// Register the adapter (called from `run_messaging` when enabled).
pub fn register(cfg: &GoogleChatConfig) {
    let resolved = cfg.resolve();
    let key = if resolved.service_account_file.is_empty() {
        eprintln!(
            "[google_chat] no service-account file configured (GOOGLE_CHAT_SERVICE_ACCOUNT_FILE) — outbound sends disabled"
        );
        None
    } else {
        match std::fs::read_to_string(&resolved.service_account_file) {
            Ok(contents) => match parse_service_account_key(&contents) {
                Ok(key) => Some(key),
                Err(e) => {
                    eprintln!("[google_chat] failed to parse service-account key: {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!(
                    "[google_chat] cannot read {}: {e} — outbound sends disabled",
                    resolved.service_account_file
                );
                None
            }
        }
    };
    let runtime = Arc::new(Runtime {
        client: reqwest::Client::builder()
            .timeout(API_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        cfg: resolved,
        key,
        token: Mutex::new(None),
        pubsub_token: Mutex::new(None),
        dedup: Mutex::new(HashMap::new()),
        threads: Mutex::new(HashMap::new()),
    });
    let _ = RUNTIME.set(runtime.clone());
    crate::messaging::register_platform_sender(
        "google_chat",
        Arc::new(GoogleChatSender {
            runtime: runtime.clone(),
        }),
    );
}

fn runtime() -> Option<Arc<Runtime>> {
    RUNTIME.get().cloned()
}

/// hermes `verify_http_event_request` — tokeninfo-based ID token
/// verification (audience + service-account email match).
pub async fn verify_http_event_request(
    client: &reqwest::Client,
    auth_header: &str,
    audience: &str,
    expected_emails_csv: &str,
) -> Result<(), String> {
    if audience.is_empty() || expected_emails_csv.trim().is_empty() {
        return Err("google_chat_http_events_not_configured".into());
    }
    let Some(token) = auth_header.strip_prefix("Bearer ").map(|t| t.trim()) else {
        return Err("missing_google_bearer".into());
    };
    if token.is_empty() {
        return Err("missing_google_bearer".into());
    }
    let resp = client
        .get(TOKENINFO_URL)
        .query(&[("id_token", token)])
        .send()
        .await
        .map_err(|e| format!("tokeninfo request: {e}"))?;
    if !resp.status().is_success() {
        return Err("invalid_google_bearer".into());
    }
    let claims: Value = resp.json().await.unwrap_or(json!({}));
    let aud = claims.get("aud").and_then(|v| v.as_str()).unwrap_or("");
    if aud != audience {
        return Err("invalid_google_bearer".into());
    }
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let expected: Vec<String> = expected_emails_csv
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if email.is_empty() || !expected.iter().any(|e| e == &email) {
        return Err("unexpected_google_bearer_identity".into());
    }
    Ok(())
}

/// Webhook response handed back to the gateway route.
pub struct GoogleChatWebhookResponse {
    pub status: u16,
    pub body: Value,
}

fn ack() -> GoogleChatWebhookResponse {
    GoogleChatWebhookResponse {
        status: 200,
        body: json!({}),
    }
}

/// Gateway webhook entry point, mounted at `/webhooks/googlechat`.
pub async fn google_chat_handle_webhook(
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    body: &[u8],
    headers: &[(String, String)],
) -> GoogleChatWebhookResponse {
    let Some(runtime) = runtime() else {
        return GoogleChatWebhookResponse {
            status: 503,
            body: json!({ "error": "google_chat adapter not registered" }),
        };
    };
    let auth_header = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    if let Err(reason) = verify_http_event_request(
        &runtime.client,
        &auth_header,
        &runtime.cfg.http_events_audience,
        &runtime.cfg.http_events_service_account,
    )
    .await
    {
        let status = if reason == "google_chat_http_events_not_configured" {
            501
        } else {
            401
        };
        return GoogleChatWebhookResponse {
            status,
            body: json!({ "error": reason }),
        };
    }
    let envelope: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            return GoogleChatWebhookResponse {
                status: 400,
                body: json!({ "error": "invalid event JSON" }),
            }
        }
    };
    process_envelope(&runtime, dispatcher, pairing, &envelope).await
}

/// Shared envelope pipeline for the HTTP webhook and the Pub/Sub pull
/// transport (hermes `dispatch_http_event` / `_on_pubsub_message`
/// converge on the same dispatch path).
async fn process_envelope(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    envelope: &Value,
) -> GoogleChatWebhookResponse {
    let event_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if event_type != "MESSAGE" {
        // ADDED_TO_SPACE / REMOVED_FROM_SPACE / CARD_CLICKED are acked.
        return ack();
    }
    let message = envelope.get("message").cloned().unwrap_or(json!({}));
    // Bot senders never loop back.
    if message
        .pointer("/sender/type")
        .and_then(|v| v.as_str())
        .map(|t| t == "BOT")
        .unwrap_or(false)
    {
        return ack();
    }
    let msg_name = message
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if runtime.is_duplicate(&msg_name).await {
        return ack();
    }
    let space = envelope
        .get("space")
        .or_else(|| message.get("space"))
        .cloned()
        .unwrap_or(json!({}));
    let space_name = space
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let space_type = space
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if space_name.is_empty() {
        return ack();
    }
    let thread_name = message
        .pointer("/thread/name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let text = message
        .get("argumentText")
        .or_else(|| message.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let sender = message.get("sender").cloned().unwrap_or(json!({}));
    let sender_id = sender
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_email = sender
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_display = sender
        .get("displayName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| sender_email.clone());
    let _is_dm = matches!(space_type, "DIRECT_MESSAGE" | "DM");

    // Allowlist ∪ pairing gate (email or user resource name).
    let authorized = runtime
        .cfg
        .allowed_users
        .iter()
        .any(|u| u == "*" || (!sender_email.is_empty() && u.eq_ignore_ascii_case(&sender_email)) || u == &sender_id);
    if !authorized {
        if let Some(store) = pairing {
            let key = if sender_email.is_empty() {
                sender_id.clone()
            } else {
                sender_email.clone()
            };
            if !store.is_approved("google_chat", &key) {
                if let Some(code_msg) =
                    crate::messaging::pairing_offer_public(store, "google_chat", &key, &sender_display)
                {
                    let _ = runtime.create_message(&space_name, &code_msg, None).await;
                }
                return ack();
            }
        } else {
            eprintln!("[google_chat] unauthorized sender {sender_email} — add to allowed_users");
            return ack();
        }
    }
    if text.is_empty() {
        return ack();
    }
    // Reply threading: keep answers in the inbound thread when one
    // exists (hermes group behavior; DM thread heuristics simplified).
    if !thread_name.is_empty() {
        runtime
            .threads
            .lock()
            .await
            .insert(space_name.clone(), thread_name.clone());
    }
    let event = MessageEvent {
        platform: "google_chat".into(),
        chat_id: space_name.clone(),
        sender_id: if sender_id.is_empty() {
            sender_email.clone()
        } else {
            sender_id
        },
        sender_name: sender_display,
        text: text.clone(),
        message_id: msg_name,
        attachments: Vec::new(),
    };
    let mut gate_check = event.clone();
    if !crate::messaging::pre_gateway_dispatch_gate_public(&mut gate_check).await {
        return ack();
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
    let reply_text = reply_text.trim().to_string();
    if !reply_text.is_empty() {
        let thread = runtime.threads.lock().await.get(&space_name).cloned();
        for chunk in crate::messaging::chunk_text(&reply_text, MAX_MESSAGE_LENGTH) {
            if let Err(e) = runtime
                .create_message(&space_name, &chunk, thread.as_deref())
                .await
            {
                eprintln!("[google_chat] reply failed: {e}");
            }
        }
    }
    ack()
}

struct GoogleChatSender {
    runtime: Arc<Runtime>,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for GoogleChatSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        let thread = self.runtime.threads.lock().await.get(chat_id).cloned();
        for chunk in crate::messaging::chunk_text(text, MAX_MESSAGE_LENGTH) {
            if let Err(e) = self
                .runtime
                .create_message(chat_id, &chunk, thread.as_deref())
                .await
            {
                eprintln!("[google_chat] send_text to {chat_id} failed: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pub/Sub pull transport (hermes streaming_pull supervisor +
// `_on_pubsub_message`, REST pull in place of gRPC)
// ---------------------------------------------------------------------------

impl Runtime {
    /// Pub/Sub-scoped service-account token (cached separately from the
    /// Chat API token).
    async fn pubsub_access_token(&self) -> Result<String, String> {
        {
            let cached = self.pubsub_token.lock().await;
            if let Some((token, expiry)) = cached.as_ref() {
                if Instant::now() < *expiry {
                    return Ok(token.clone());
                }
            }
        }
        let key = self
            .key
            .as_ref()
            .ok_or("no service-account key configured")?;
        let assertion = build_jwt_assertion_scoped(key, PUBSUB_SCOPE)?;
        let resp = self
            .client
            .post(&key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("pubsub token request: {e}"))?;
        let status = resp.status();
        let payload: Value = resp.json().await.unwrap_or(json!({}));
        if status.as_u16() >= 400 {
            return Err(format!("pubsub token request failed ({status}): {payload}"));
        }
        let token = payload
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or("pubsub token response missing access_token")?
            .to_string();
        let expires_in = payload
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);
        *self.pubsub_token.lock().await = Some((
            token.clone(),
            Instant::now() + Duration::from_secs(expires_in.saturating_sub(300)),
        ));
        Ok(token)
    }
}

/// Normalize the subscription to the `projects/<p>/subscriptions/<s>`
/// resource path (accepts bare names when the project prefix is
/// already present).
pub fn pubsub_subscription_path(subscription: &str) -> String {
    subscription.trim().to_string()
}

/// hermes `_extract_message_payload` — detect the three accepted
/// envelope formats and return `(message, space)`.
pub fn extract_message_payload(envelope: &Value, ce_type: &str) -> Option<(Value, Value)> {
    // Format 1 — Workspace Add-ons (canonical, ce-type driven).
    if let Some(wrapper) = envelope.pointer("/chat/messagePayload") {
        let msg = wrapper.get("message").cloned().unwrap_or(json!({}));
        let space = wrapper
            .get("space")
            .cloned()
            .or_else(|| msg.get("space").cloned())
            .unwrap_or(json!({}));
        return Some((msg, space));
    }
    // Format 2 — native Chat API Pub/Sub.
    if envelope.get("message").map(|v| v.is_object()).unwrap_or(false) {
        if envelope.get("type").and_then(|v| v.as_str()) != Some("MESSAGE") {
            return None;
        }
        let msg = envelope.get("message").cloned().unwrap_or(json!({}));
        let space = envelope
            .get("space")
            .cloned()
            .or_else(|| msg.get("space").cloned())
            .unwrap_or(json!({}));
        return Some((msg, space));
    }
    // Format 3 — relay / flat.
    if envelope.get("event_type").is_some() || envelope.get("sender_email").is_some() {
        let event_type = envelope
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("MESSAGE");
        if event_type != "MESSAGE" {
            return None;
        }
        let sender_email = envelope
            .get("sender_email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let sender_display = envelope
            .get("sender_display_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if sender_email.is_empty() {
                    "Unknown".to_string()
                } else {
                    sender_email.clone()
                }
            });
        let surrogate = format!(
            "users/relay-{}",
            if sender_email.is_empty() { "unknown".to_string() } else {
                sender_email.replace('@', "_at_").replace('.', "_")
            }
        );
        let text = envelope
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sender_type = envelope
            .get("sender_type")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_uppercase())
            .filter(|s| s == "HUMAN" || s == "BOT")
            .unwrap_or_else(|| "HUMAN".into());
        let mut msg = json!({
            "name": envelope.get("message_name").and_then(|v| v.as_str()).unwrap_or(""),
            "sender": {
                "name": surrogate,
                "email": sender_email,
                "displayName": sender_display,
                "type": sender_type,
            },
            "text": text,
            "argumentText": text,
        });
        if let Some(thread_name) = envelope
            .get("thread_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            msg["thread"] = json!({ "name": thread_name });
        }
        let space = json!({
            "name": envelope.get("space_name").and_then(|v| v.as_str()).unwrap_or(""),
            "spaceType": envelope.get("space_type").and_then(|v| v.as_str()).unwrap_or("SPACE"),
        });
        return Some((msg, space));
    }
    let _ = ce_type;
    None
}

/// One Pub/Sub delivery (hermes `_on_pubsub_message`): membership and
/// card events are logged + acked; message payloads normalize into the
/// webhook envelope shape and flow through `process_envelope`.
async fn process_pubsub_message(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    envelope: &Value,
    attributes: &Value,
) {
    let ce_type = attributes
        .get("ce-type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if ce_type.contains("membership") || ce_type.contains("MEMBERSHIP") {
        if ce_type.contains("created") {
            if let Some(name) = envelope.pointer("/chat/membershipPayload/membership/member/name")
                .and_then(|v| v.as_str())
            {
                let member_type = envelope
                    .pointer("/chat/membershipPayload/membership/member/type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if member_type == "BOT" {
                    eprintln!("[google_chat] ADDED_TO_SPACE (bot id {name})");
                }
            }
        } else {
            eprintln!("[google_chat] REMOVED_FROM_SPACE");
        }
        return;
    }
    if ce_type.contains("widget") || ce_type.to_lowercase().contains("card") {
        eprintln!("[google_chat] card/widget event ack'd (v2 feature, deferred)");
        return;
    }
    let Some((msg, space)) = extract_message_payload(envelope, &ce_type) else {
        return;
    };
    // Synthesize the webhook-shaped envelope; BOT filter + dedup +
    // gating happen inside process_envelope.
    let mut message = msg;
    if let Some(obj) = message.as_object_mut() {
        if !obj.contains_key("space") && space.is_object() {
            obj.insert("space".into(), space.clone());
        }
    }
    let normalized = json!({
        "type": "MESSAGE",
        "message": message,
        "space": space,
    });
    process_envelope(runtime, dispatcher, pairing, &normalized).await;
}

/// One pull batch; errors classify as fatal (auth/permission) or
/// retryable (hermes supervisor exceptions).
async fn pull_batch(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    subscription: &str,
) -> Result<(), String> {
    let token = runtime.pubsub_access_token().await?;
    let path = pubsub_subscription_path(subscription);
    let resp = runtime
        .client
        .post(format!("{PUBSUB_API_BASE}/{path}:pull"))
        .bearer_auth(&token)
        .json(&json!({ "maxMessages": PULL_MAX_MESSAGES }))
        .timeout(API_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("pull request: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        return Err("unauthenticated".into());
    }
    if status == 403 {
        return Err("permission_denied".into());
    }
    let body_text = resp.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(format!(
            "pull HTTP {status}: {}",
            &body_text[..body_text.len().min(200)]
        ));
    }
    let payload: Value = serde_json::from_str(&body_text).unwrap_or(json!({}));
    let received = payload
        .get("receivedMessages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if received.is_empty() {
        // REST pull pacing — streaming pull blocks server-side; the
        // REST equivalent must not hot-spin on an idle subscription.
        tokio::time::sleep(PULL_IDLE_DELAY).await;
        return Ok(());
    }
    let mut ack_ids: Vec<String> = Vec::new();
    for received_msg in &received {
        let ack_id = received_msg
            .get("ackId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let data_b64 = received_msg
            .pointer("/message/data")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let envelope: Value = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(json!({}));
        let attributes = received_msg
            .pointer("/message/attributes")
            .cloned()
            .unwrap_or(json!({}));
        process_pubsub_message(runtime, dispatcher, pairing, &envelope, &attributes).await;
        // hermes acks even malformed deliveries to avoid redelivery
        // loops.
        if !ack_id.is_empty() {
            ack_ids.push(ack_id);
        }
    }
    if !ack_ids.is_empty() {
        let token = runtime.pubsub_access_token().await?;
        let resp = runtime
            .client
            .post(format!("{PUBSUB_API_BASE}/{path}:acknowledge"))
            .bearer_auth(token)
            .json(&json!({ "ackIds": ack_ids }))
            .timeout(API_TIMEOUT)
            .send()
            .await;
        if let Ok(resp) = resp {
            if !resp.status().is_success() {
                eprintln!("[google_chat] pubsub acknowledge failed: HTTP {}", resp.status());
            }
        }
    }
    Ok(())
}

fn random_fraction() -> f64 {
    let mut bytes = [0u8; 4];
    crate::feishu::fill_random_bytes(&mut bytes);
    u32::from_le_bytes(bytes) as f64 / u32::MAX as f64
}

/// Pub/Sub pull supervisor (hermes `_run_supervisor`): full-jitter
/// exponential backoff, fatal on auth/permission errors or after 10
/// failed attempts.
pub async fn run_pubsub(
    dispatcher: Arc<Dispatcher>,
    pairing: Option<Arc<crate::pairing::PairingStore>>,
) {
    let Some(runtime) = runtime() else {
        return;
    };
    let subscription = runtime.cfg.pubsub_subscription.trim().to_string();
    if subscription.is_empty() {
        return;
    }
    if runtime.key.is_none() {
        eprintln!(
            "[google_chat] pubsub subscription configured but no service-account key — pull transport not started"
        );
        return;
    }
    eprintln!("[google_chat] pub/sub pull transport started on {subscription}");
    let mut attempt: u32 = 0;
    loop {
        match pull_batch(&runtime, &dispatcher, pairing.as_deref(), &subscription).await {
            Ok(()) => {
                if attempt > 0 {
                    eprintln!("[google_chat] pub/sub stream recovered after {attempt} attempt(s)");
                }
                attempt = 0;
            }
            Err(e) => {
                if e == "unauthenticated" {
                    eprintln!(
                        "[google_chat] pub/sub authentication failed (SA key invalid/revoked) — transport stopped"
                    );
                    return;
                }
                if e == "permission_denied" {
                    eprintln!(
                        "[google_chat] SA lacks pubsub.subscriber on the subscription — transport stopped"
                    );
                    return;
                }
                attempt += 1;
                eprintln!(
                    "[google_chat] pub/sub pull failed (attempt {attempt}/{MAX_RECONNECT_ATTEMPTS}): {e}"
                );
                if attempt >= MAX_RECONNECT_ATTEMPTS {
                    eprintln!(
                        "[google_chat] pub/sub reconnect failed {attempt} times; giving up"
                    );
                    return;
                }
                let delay_ms = RECONNECT_MAX_DELAY_MS
                    .min(RECONNECT_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow(attempt - 1)));
                let jitter_ms = (random_fraction() * delay_ms as f64) as u64;
                tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_account_key_parsing() {
        let key_json = json!({
            "type": "service_account",
            "client_email": "bot@project.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token",
            "private_key": "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n",
        })
        .to_string();
        let key = parse_service_account_key(&key_json).unwrap();
        assert_eq!(key.client_email, "bot@project.iam.gserviceaccount.com");
        assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
        assert!(key.private_key_pem.contains("PRIVATE KEY"));
    }

    #[test]
    fn service_account_key_missing_fields() {
        assert!(parse_service_account_key("{}").is_err());
        assert!(parse_service_account_key("not json").is_err());
        let no_pk = json!({"client_email": "a@b.c"}).to_string();
        assert!(parse_service_account_key(&no_pk).is_err());
    }

    #[test]
    fn jwt_assertion_needs_valid_rsa_key() {
        let key = ServiceAccountKey {
            client_email: "bot@p.iam.gserviceaccount.com".into(),
            token_uri: "https://oauth2.googleapis.com/token".into(),
            private_key_pem: "not a pem".into(),
        };
        assert!(build_jwt_assertion(&key).is_err());
    }

    #[tokio::test]
    async fn dedup_window() {
        let runtime = Runtime {
            client: reqwest::Client::new(),
            cfg: GoogleChatConfig::default().resolve(),
            key: None,
            token: Mutex::new(None),
            pubsub_token: Mutex::new(None),
            dedup: Mutex::new(HashMap::new()),
            threads: Mutex::new(HashMap::new()),
        };
        assert!(!runtime.is_duplicate("spaces/a/messages/1").await);
        assert!(runtime.is_duplicate("spaces/a/messages/1").await);
        assert!(!runtime.is_duplicate("spaces/a/messages/2").await);
    }

    #[tokio::test]
    async fn verify_requires_configuration() {
        let client = reqwest::Client::new();
        let err = verify_http_event_request(&client, "Bearer x", "", "").await;
        assert_eq!(err.unwrap_err(), "google_chat_http_events_not_configured");
        let err = verify_http_event_request(&client, "Basic x", "aud", "a@b.c").await;
        assert_eq!(err.unwrap_err(), "missing_google_bearer");
        let err = verify_http_event_request(&client, "Bearer ", "aud", "a@b.c").await;
        assert_eq!(err.unwrap_err(), "missing_google_bearer");
    }

    #[test]
    fn envelope_type_routing() {
        // Only MESSAGE dispatches; the rest ack.
        for (ty, dispatches) in [
            ("MESSAGE", true),
            ("ADDED_TO_SPACE", false),
            ("REMOVED_FROM_SPACE", false),
            ("CARD_CLICKED", false),
        ] {
            assert_eq!(ty == "MESSAGE", dispatches);
        }
    }

    #[test]
    fn sender_bot_skip_and_text_preference() {
        let msg: Value = serde_json::from_str(
            r#"{"sender":{"type":"BOT"},"text":"echo","argumentText":"arg"}"#,
        )
        .unwrap();
        assert!(msg.pointer("/sender/type").and_then(|v| v.as_str()) == Some("BOT"));
        let text = msg
            .get("argumentText")
            .or_else(|| msg.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(text, "arg");
    }

    #[test]
    fn dm_vs_group_space_types() {
        for (space_type, is_dm) in [
            ("DIRECT_MESSAGE", true),
            ("DM", true),
            ("ROOM", false),
            ("SPACE", false),
        ] {
            assert_eq!(matches!(space_type, "DIRECT_MESSAGE" | "DM"), is_dm);
        }
    }

    #[test]
    fn resolve_env_precedence() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::set_var("GOOGLE_CHAT_ALLOWED_USERS", "a@x.com, b@x.com");
        std::env::set_var("GOOGLE_CHAT_HTTP_EVENTS_AUDIENCE", "https://gw.example.com/webhooks/googlechat");
        let cfg = GoogleChatConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(
            resolved.allowed_users,
            vec!["a@x.com".to_string(), "b@x.com".to_string()]
        );
        assert_eq!(
            resolved.http_events_audience,
            "https://gw.example.com/webhooks/googlechat"
        );
        std::env::remove_var("GOOGLE_CHAT_ALLOWED_USERS");
        std::env::remove_var("GOOGLE_CHAT_HTTP_EVENTS_AUDIENCE");
    }

    #[test]
    fn chunk_limit_matches_hermes() {
        assert_eq!(MAX_MESSAGE_LENGTH, 4000);
        assert_eq!(CHAT_SCOPE, "https://www.googleapis.com/auth/chat.bot");
        assert_eq!(PUBSUB_SCOPE, "https://www.googleapis.com/auth/pubsub");
        assert_eq!(MAX_RECONNECT_ATTEMPTS, 10);
        assert_eq!(PULL_MAX_MESSAGES, 1);
    }

    #[test]
    fn extract_workspace_addons_format() {
        let envelope = json!({
            "chat": {
                "messagePayload": {
                    "message": { "name": "spaces/X/messages/1", "text": "hi" },
                    "space": { "name": "spaces/X" },
                },
            },
        });
        let (msg, space) = extract_message_payload(&envelope, "google.workspace.chat.message.v1.created").unwrap();
        assert_eq!(msg["text"], "hi");
        assert_eq!(space["name"], "spaces/X");
    }

    #[test]
    fn extract_native_chat_api_format() {
        let envelope = json!({
            "type": "MESSAGE",
            "message": { "text": "hello", "space": { "name": "spaces/Y" } },
            "space": { "name": "spaces/Y" },
        });
        let (msg, space) = extract_message_payload(&envelope, "").unwrap();
        assert_eq!(msg["text"], "hello");
        assert_eq!(space["name"], "spaces/Y");
        // Non-MESSAGE type is dropped.
        let envelope = json!({ "type": "ADDED_TO_SPACE", "message": {} });
        assert!(extract_message_payload(&envelope, "").is_none());
    }

    #[test]
    fn extract_relay_flat_format() {
        let envelope = json!({
            "event_type": "MESSAGE",
            "sender_email": "alice@example.com",
            "sender_display_name": "Alice",
            "text": "ping",
            "space_name": "spaces/Z",
            "thread_name": "spaces/Z/threads/T",
            "message_name": "spaces/Z/messages/M",
        });
        let (msg, space) = extract_message_payload(&envelope, "").unwrap();
        assert_eq!(msg["text"], "ping");
        assert_eq!(msg["sender"]["email"], "alice@example.com");
        // Default sender_type is HUMAN.
        assert_eq!(msg["sender"]["type"], "HUMAN");
        assert_eq!(msg["sender"]["name"], "users/relay-alice_at_example_com");
        assert_eq!(msg["thread"]["name"], "spaces/Z/threads/T");
        assert_eq!(space["name"], "spaces/Z");
        // Declared BOT sender_type is honored (self-filter).
        let envelope = json!({ "event_type": "MESSAGE", "sender_email": "bot@x.com", "text": "t", "sender_type": "bot" });
        let (msg, _) = extract_message_payload(&envelope, "").unwrap();
        assert_eq!(msg["sender"]["type"], "BOT");
    }

    #[test]
    fn extract_unknown_format_returns_none() {
        assert!(extract_message_payload(&json!({}), "").is_none());
        assert!(extract_message_payload(&json!({"foo": "bar"}), "").is_none());
    }

    #[test]
    fn resolve_pubsub_subscription_env() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::set_var(
            "GOOGLE_CHAT_PUBSUB_SUBSCRIPTION",
            "projects/p/subscriptions/s",
        );
        let cfg = GoogleChatConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(resolved.pubsub_subscription, "projects/p/subscriptions/s");
        std::env::remove_var("GOOGLE_CHAT_PUBSUB_SUBSCRIPTION");
    }
}
