//! DingTalk platform adapter — port of hermes `plugins/platforms/dingtalk`
//! @ v2026.8.3 (adapter.py).
//!
//! Uses DingTalk **Stream Mode** without the Python SDK: the gateway
//! connection is opened via `POST /v1.0/gateway/connections/open`
//! (client id + secret + a CALLBACK subscription on
//! `/v1.0/im/bot/messages/get`), then a WebSocket to the returned
//! endpoint carries JSON frames that are ACKed hermes-style. Replies go
//! out through the per-chat `sessionWebhook` as markdown.
//!
//! Intake parity: `allowed_users` (staff_id/sender_id, `*` wildcard),
//! hard `allowed_chats` group whitelist, `require_mention` (hermes
//! default **false** for DingTalk) with `isInAtList` detection, regex
//! `mention_patterns` wake words, and `free_response_chats`. Inbound
//! media arrives as `downloadCode` references resolved through
//! `/v1.0/robot/messageFiles/download` with an OAuth2 access token and
//! cached into the media cache.
//!
//! Known differences: AI streaming cards (`card_1_0` SDK), Thinking/Done
//! emoji reactions, and message editing are not ported — replies are
//! plain webhook markdown.

use crate::messaging::{Dispatcher, MediaAttachment, MessageEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const API_BASE: &str = "https://api.dingtalk.com";
const BOT_TOPIC: &str = "/v1.0/im/bot/messages/get";
/// hermes `MAX_MESSAGE_LENGTH` for DingTalk markdown.
const MAX_MESSAGE_LENGTH: usize = 20_000;
const RECONNECT_BACKOFF: &[u64] = &[2, 5, 10, 30, 60];
const SESSION_WEBHOOKS_MAX: usize = 500;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WEBHOOK_SEND_TIMEOUT: Duration = Duration::from_secs(15);
const DEDUP_WINDOW_SECS: u64 = 300;
const DEDUP_MAX_SIZE: usize = 1000;
/// Access tokens are valid ~7200 s; refresh with a safety margin.
const TOKEN_REFRESH_SECS: u64 = 6600;

/// `[messaging.dingtalk]` — DingTalk Stream Mode adapter (hermes
/// `platforms.dingtalk` plugin config + `DINGTALK_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DingTalkConfig {
    pub enabled: bool,
    /// App key (fallback `DINGTALK_CLIENT_ID`).
    pub client_id: String,
    /// App secret (fallback `DINGTALK_CLIENT_SECRET`).
    pub client_secret: String,
    /// Require @mention / wake word in group chats. Hermes defaults this
    /// to FALSE for DingTalk.
    pub require_mention: bool,
    /// Conversation ids that skip the mention gate.
    pub free_response_chats: Vec<String>,
    /// When non-empty, group messages outside this set are ignored.
    pub allowed_chats: Vec<String>,
    /// staff_id/sender_id allowlist (`*` = anyone; empty = anyone —
    /// hermes `_is_user_allowed`).
    pub allowed_users: Vec<String>,
    /// Regex wake words that trigger the bot in groups.
    pub mention_patterns: Vec<String>,
}

impl Default for DingTalkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            client_secret: String::new(),
            require_mention: false,
            free_response_chats: Vec::new(),
            allowed_chats: Vec::new(),
            allowed_users: Vec::new(),
            mention_patterns: Vec::new(),
        }
    }
}

fn env_trim(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_bool(name: &str, default: bool) -> bool {
    match env_trim(name) {
        Some(v) => matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on"),
        None => default,
    }
}

fn env_csv(name: &str) -> Option<Vec<String>> {
    env_trim(name).map(|raw| {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

#[derive(Debug, Clone)]
pub struct ResolvedDingTalk {
    pub client_id: String,
    pub client_secret: String,
    pub require_mention: bool,
    pub free_response_chats: Vec<String>,
    pub allowed_chats: Vec<String>,
    pub allowed_users: Vec<String>,
    pub mention_patterns: Vec<String>,
}

impl DingTalkConfig {
    pub fn resolve(&self) -> ResolvedDingTalk {
        let mention_patterns = match env_trim("DINGTALK_MENTION_PATTERNS") {
            Some(raw) => {
                // hermes: JSON array first, then newline/comma split.
                if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
                    list
                } else {
                    raw.split(|c| c == '\n' || c == ',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
            }
            None => self.mention_patterns.clone(),
        };
        ResolvedDingTalk {
            client_id: env_trim("DINGTALK_CLIENT_ID")
                .unwrap_or_else(|| self.client_id.trim().to_string()),
            client_secret: env_trim("DINGTALK_CLIENT_SECRET")
                .unwrap_or_else(|| self.client_secret.trim().to_string()),
            require_mention: env_bool("DINGTALK_REQUIRE_MENTION", self.require_mention),
            free_response_chats: env_csv("DINGTALK_FREE_RESPONSE_CHATS")
                .unwrap_or_else(|| self.free_response_chats.clone()),
            allowed_chats: env_csv("DINGTALK_ALLOWED_CHATS")
                .unwrap_or_else(|| self.allowed_chats.clone()),
            allowed_users: env_csv("DINGTALK_ALLOWED_USERS")
                .unwrap_or_else(|| self.allowed_users.clone()),
            mention_patterns,
        }
    }
}

struct Runtime {
    cfg: ResolvedDingTalk,
    client: reqwest::Client,
    /// chat_id → (session_webhook, expired_ms).
    session_webhooks: Mutex<HashMap<String, (String, u64)>>,
    dedup: Mutex<HashMap<String, u64>>,
    access_token: Mutex<(String, std::time::Instant)>,
    mention_regexes: Vec<regex::Regex>,
}

impl Runtime {
    fn is_user_allowed(&self, sender_id: &str, sender_staff_id: &str) -> bool {
        if self.cfg.allowed_users.is_empty()
            || self.cfg.allowed_users.iter().any(|u| u == "*")
        {
            return true;
        }
        let lower: Vec<String> = self
            .cfg
            .allowed_users
            .iter()
            .map(|u| u.to_lowercase())
            .collect();
        [sender_id, sender_staff_id]
            .iter()
            .any(|id| !id.is_empty() && lower.contains(&id.to_lowercase()))
    }

    fn matches_mention_patterns(&self, text: &str) -> bool {
        self.mention_regexes.iter().any(|re| re.is_match(text))
    }

    /// hermes `_should_process_message` group gate.
    fn should_process(&self, text: &str, is_group: bool, chat_id: &str, in_at_list: bool) -> bool {
        if !is_group {
            return true;
        }
        if !self.cfg.allowed_chats.is_empty() && !self.cfg.allowed_chats.contains(&chat_id.to_string()) {
            return false;
        }
        if self.cfg.free_response_chats.contains(&chat_id.to_string()) {
            return true;
        }
        if !self.cfg.require_mention {
            return true;
        }
        in_at_list || self.matches_mention_patterns(text)
    }

    async fn is_duplicate(&self, msg_id: &str) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut dedup = self.dedup.lock().await;
        dedup.retain(|_, ts| now.saturating_sub(*ts) < DEDUP_WINDOW_SECS);
        if dedup.contains_key(msg_id) {
            return true;
        }
        if dedup.len() >= DEDUP_MAX_SIZE {
            let mut entries: Vec<(String, u64)> = dedup.drain().collect();
            entries.sort_by_key(|(_, ts)| *ts);
            entries.truncate(entries.len() / 2);
            *dedup = entries.into_iter().collect();
        }
        dedup.insert(msg_id.to_string(), now);
        false
    }

    /// Cache the inbound session webhook (hermes `_session_webhooks`,
    /// LRU-ish cap + 5-minute expiry margin).
    async fn store_webhook(&self, chat_id: &str, webhook: &str, expired_ms: u64) {
        if !is_dingtalk_webhook_url(webhook) {
            return;
        }
        let mut map = self.session_webhooks.lock().await;
        if map.len() >= SESSION_WEBHOOKS_MAX {
            let oldest = map.keys().next().cloned();
            if let Some(key) = oldest {
                map.remove(&key);
            }
        }
        map.insert(chat_id.to_string(), (webhook.to_string(), expired_ms));
    }

    fn valid_webhook<'a>(map: &'a HashMap<String, (String, u64)>, chat_id: &str) -> Option<&'a String> {
        let (webhook, expired_ms) = map.get(chat_id)?;
        if *expired_ms > 0 {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let margin_ms: u64 = 5 * 60 * 1000;
            if now_ms + margin_ms >= *expired_ms {
                return None;
            }
        }
        Some(webhook)
    }

    /// OAuth2 access token for media download (cached).
    async fn get_access_token(&self) -> Result<String, String> {
        let (token, fetched_at) = self.access_token.lock().await.clone();
        if !token.is_empty() && fetched_at.elapsed() < Duration::from_secs(TOKEN_REFRESH_SECS) {
            return Ok(token);
        }
        let resp = self
            .client
            .post(format!("{API_BASE}/v1.0/oauth2/accessToken"))
            .json(&json!({"appKey": self.cfg.client_id, "appSecret": self.cfg.client_secret}))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("accessToken: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        if status >= 400 {
            return Err(format!("accessToken → {status}: {body}"));
        }
        let value: Value = serde_json::from_str(&body).map_err(|e| format!("accessToken JSON: {e}"))?;
        let token = value
            .get("accessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "accessToken missing in response".to_string())?
            .to_string();
        *self.access_token.lock().await = (token.clone(), std::time::Instant::now());
        Ok(token)
    }

    /// Resolve a `downloadCode` to a URL and cache the bytes.
    async fn download_media_code(
        &self,
        download_code: &str,
        robot_code: &str,
        filename_hint: &str,
    ) -> Option<MediaAttachment> {
        let token = self.get_access_token().await.ok()?;
        let resp = self
            .client
            .post(format!("{API_BASE}/v1.0/robot/messageFiles/download"))
            .header("x-acs-dingtalk-access-token", &token)
            .json(&json!({"downloadCode": download_code, "robotCode": robot_code}))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            eprintln!("[dingtalk] media resolve failed: HTTP {}", resp.status());
            return None;
        }
        let value: Value = resp.json().await.ok()?;
        let url = value.get("downloadUrl").and_then(|v| v.as_str())?;
        let file_resp = self.client.get(url).timeout(REQUEST_TIMEOUT).send().await.ok()?;
        if !file_resp.status().is_success() {
            return None;
        }
        let bytes = file_resp.bytes().await.ok()?.to_vec();
        let mime = mime_from_filename(filename_hint);
        let path = crate::media_cache::cache_media_bytes(
            &crate::config::ulnclaw_home(),
            &bytes,
            &mime,
            filename_hint,
        )
        .ok()?;
        Some(MediaAttachment {
            path,
            mime,
            bytes: bytes.len() as u64,
            original_name: filename_hint.to_string(),
        })
    }

    /// Send markdown via session webhook.
    async fn send_markdown(&self, chat_id: &str, content: &str) -> Result<(), String> {
        let webhook = {
            let map = self.session_webhooks.lock().await;
            Self::valid_webhook(&map, chat_id).cloned()
        };
        let Some(webhook) = webhook else {
            return Err("no valid session_webhook (reply must follow an incoming message)".into());
        };
        let normalized = normalize_markdown(content);
        for chunk in crate::messaging::chunk_text(&normalized, MAX_MESSAGE_LENGTH) {
            let payload = json!({
                "msgtype": "markdown",
                "markdown": {"title": "Ulnclaw", "text": chunk},
            });
            let resp = self
                .client
                .post(&webhook)
                .json(&payload)
                .timeout(WEBHOOK_SEND_TIMEOUT)
                .send()
                .await
                .map_err(|e| format!("webhook send: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("webhook HTTP error: {body}"));
            }
        }
        Ok(())
    }
}

/// hermes `_DINGTALK_WEBHOOK_RE`.
pub fn is_dingtalk_webhook_url(url: &str) -> bool {
    url.starts_with("https://api.dingtalk.com/") || url.starts_with("https://oapi.dingtalk.com/")
}

fn mime_from_filename(filename: &str) -> String {
    let mime = crate::media_cache::mime_for_ext(std::path::Path::new(filename));
    if mime == "application/octet-stream" {
        return mime;
    }
    mime
}

/// hermes `_normalize_markdown`: blank line before numbered lists, dedent
/// fenced code blocks.
pub fn normalize_markdown(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let numbered = regex::Regex::new(r"^\d+\.\s").unwrap();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if numbered.is_match(line.trim()) && i > 0 {
            let prev = lines[i - 1];
            if !prev.trim().is_empty() && !numbered.is_match(prev.trim()) {
                out.push(String::new());
            }
        }
        let trimmed = line.trim_start();
        let pushed = if trimmed.starts_with("```") && trimmed.len() != line.len() {
            trimmed.to_string()
        } else {
            line.to_string()
        };
        out.push(pushed);
    }
    out.join("\n")
}

/// Entry point spawned by `run_messaging`.
pub async fn run(
    cfg: DingTalkConfig,
    dispatcher: Arc<Dispatcher>,
    pairing: Option<Arc<crate::pairing::PairingStore>>,
) {
    let resolved = cfg.resolve();
    if resolved.client_id.is_empty() || resolved.client_secret.is_empty() {
        eprintln!(
            "[dingtalk] disabled: set [messaging.dingtalk] client_id/client_secret or DINGTALK_CLIENT_ID/DINGTALK_CLIENT_SECRET"
        );
        return;
    }
    let mention_regexes: Vec<regex::Regex> = resolved
        .mention_patterns
        .iter()
        .filter_map(|p| match regex::Regex::new(p) {
            Ok(re) => Some(re),
            Err(e) => {
                eprintln!("[dingtalk] invalid mention pattern '{p}': {e}");
                None
            }
        })
        .collect();
    let runtime = Arc::new(Runtime {
        cfg: resolved,
        client: reqwest::Client::new(),
        session_webhooks: Mutex::new(HashMap::new()),
        dedup: Mutex::new(HashMap::new()),
        access_token: Mutex::new((String::new(), std::time::Instant::now())),
        mention_regexes,
    });
    crate::messaging::register_platform_sender(
        "dingtalk",
        Arc::new(DingTalkSender {
            runtime: runtime.clone(),
        }),
    );

    let mut backoff_idx: usize = 0;
    loop {
        match stream_session(&runtime, &dispatcher, &pairing).await {
            Ok(()) => backoff_idx = 0,
            Err(e) => eprintln!("[dingtalk] stream error: {e}"),
        }
        let delay = RECONNECT_BACKOFF[backoff_idx.min(RECONNECT_BACKOFF.len() - 1)];
        eprintln!("[dingtalk] reconnecting in {delay}s");
        tokio::time::sleep(Duration::from_secs(delay)).await;
        backoff_idx += 1;
    }
}

/// Open the gateway connection and run one WebSocket session.
async fn stream_session(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
) -> Result<(), String> {
    use futures::StreamExt;

    let resp = runtime
        .client
        .post(format!("{API_BASE}/v1.0/gateway/connections/open"))
        .json(&json!({
            "clientId": runtime.cfg.client_id,
            "clientSecret": runtime.cfg.client_secret,
            "subscriptions": [{"type": "CALLBACK", "topic": BOT_TOPIC}],
            "ua": format!("ulnclaw/{}", env!("CARGO_PKG_VERSION")),
        }))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("connections/open: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(format!("connections/open → {status}: {body}"));
    }
    let open: Value = serde_json::from_str(&body).map_err(|e| format!("open JSON: {e}"))?;
    let endpoint = open
        .get("endpoint")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "connections/open: no endpoint".to_string())?;
    let ticket = open
        .get("ticket")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ws_url = format!("{endpoint}?ticket={ticket}");

    let (ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("ws connect: {e}"))?;
    eprintln!("[dingtalk] stream connected");
    let (mut sink, mut stream) = ws.split();

    while let Some(message) = stream.next().await {
        let message = match message {
            Ok(m) => m,
            Err(e) => return Err(format!("ws: {e}")),
        };
        let text = match message {
            tokio_tungstenite::tungstenite::Message::Text(t) => t,
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                return Err("ws closed by server".into())
            }
            _ => continue,
        };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let frame_type = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let headers = frame.get("headers").cloned().unwrap_or(json!({}));
        let message_id = headers
            .get("messageId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let topic = headers.get("topic").and_then(|v| v.as_str()).unwrap_or("");

        match frame_type {
            "SYSTEM" => {
                // ping/disconnect frames: ack and keep going.
                send_ack(&mut sink, &message_id, "").await;
                if topic == "disconnect" {
                    return Err("server requested disconnect".into());
                }
            }
            "CALLBACK" => {
                let data_str = frame
                    .get("data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // ACK first so the SDK-style heartbeat never blocks
                // (hermes `_IncomingHandler.process` dispatches in a
                // background task).
                send_ack(&mut sink, &message_id, "").await;
                if topic == BOT_TOPIC && !data_str.is_empty() {
                    let payload: Value = match serde_json::from_str(&data_str) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    handle_bot_message(runtime, dispatcher, pairing, &payload).await;
                }
            }
            _ => {}
        }
    }
    Err("ws stream ended".into())
}

async fn send_ack(
    sink: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
    message_id: &str,
    data: &str,
) {
    use futures::SinkExt;
    let ack = json!({
        "code": 200,
        "headers": {"messageId": message_id, "contentType": "application/json"},
        "message": "OK",
        "data": data,
    });
    let _ = sink
        .send(tokio_tungstenite::tungstenite::Message::Text(
            ack.to_string(),
        ))
        .await;
}

/// hermes `_on_message` — gating, webhook caching, media resolution,
/// dispatch.
async fn handle_bot_message(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
    payload: &Value,
) {
    let msg_id = payload
        .get("msgId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    if runtime.is_duplicate(&msg_id).await {
        return;
    }

    let conversation_id = payload
        .get("conversationId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let conversation_type = payload
        .get("conversationType")
        .and_then(|v| v.as_str())
        .unwrap_or("1");
    let is_group = conversation_type == "2";
    let sender_id = payload
        .get("senderId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_nick = payload
        .get("senderNick")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| sender_id.clone());
    let sender_staff_id = payload
        .get("senderStaffId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let robot_code = payload
        .get("robotCode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let chat_id = if conversation_id.is_empty() {
        sender_id.clone()
    } else {
        conversation_id.clone()
    };
    if chat_id.is_empty() {
        return;
    }

    if !runtime.is_user_allowed(&sender_id, &sender_staff_id) {
        eprintln!("[dingtalk] dropping message from non-allowlisted user {sender_staff_id}/{sender_id}");
        return;
    }

    let text = extract_text(payload);
    let in_at_list = payload
        .get("isInAtList")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !runtime.should_process(&text, is_group, &chat_id, in_at_list) {
        return;
    }

    // DM pairing gate (hermes leaves DMs unrestricted apart from
    // allowed_users; ulnclaw adds the pairing union like the other
    // adapters).
    if !is_group {
        let allowed = runtime.cfg.allowed_users.iter().any(|u| u == "*")
            || runtime.is_user_allowed(&sender_id, &sender_staff_id);
        if !allowed {
            if let Some(store) = pairing {
                if !store.is_approved("dingtalk", &sender_id) {
                    if let Some(code_msg) = crate::messaging::pairing_offer_public(
                        store.as_ref(),
                        "dingtalk",
                        &sender_id,
                        &sender_nick,
                    ) {
                        let _ = send_dingtalk_text(runtime, &chat_id, payload, &code_msg).await;
                    }
                    return;
                }
            }
        }
    }

    let session_webhook = payload
        .get("sessionWebhook")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expired = payload
        .get("sessionWebhookExpiredMilli")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    runtime.store_webhook(&chat_id, session_webhook, expired).await;

    // Resolve media download codes.
    let attachments: Vec<MediaAttachment> =
        collect_media_refs(runtime, payload, &robot_code)
            .await
            .into_iter()
            .flatten()
            .collect();

    if text.trim().is_empty() && attachments.is_empty() {
        return;
    }

    let mut event = MessageEvent {
        platform: "dingtalk".into(),
        chat_id: chat_id.clone(),
        sender_id: sender_id.clone(),
        sender_name: sender_nick,
        text,
        message_id: msg_id,
        attachments,
    };
    if !crate::messaging::pre_gateway_dispatch_gate_public(&mut event).await {
        return;
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
    let (reply_text, _media_paths) = crate::messaging::extract_media_tags(&full);
    // DingTalk session webhooks only carry text/markdown — media paths
    // degrade to their path lines (hermes sends image URLs in markdown;
    // local files have no public URL to link).
    if !reply_text.trim().is_empty() {
        if let Err(e) = runtime.send_markdown(&chat_id, &reply_text).await {
            eprintln!("[dingtalk] reply failed: {e}");
        }
    }
}

/// Outbound text for pairing offers before any webhook may be cached:
/// uses the inbound message's own sessionWebhook directly.
async fn send_dingtalk_text(
    runtime: &Arc<Runtime>,
    _chat_id: &str,
    payload: &Value,
    text: &str,
) -> Result<(), String> {
    let webhook = payload
        .get("sessionWebhook")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !is_dingtalk_webhook_url(webhook) {
        return Err("no session webhook".into());
    }
    let normalized = normalize_markdown(text);
    let payload = json!({
        "msgtype": "markdown",
        "markdown": {"title": "Ulnclaw", "text": normalized},
    });
    let resp = runtime
        .client
        .post(webhook)
        .json(&payload)
        .timeout(WEBHOOK_SEND_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("webhook send: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("webhook HTTP {}", resp.status()))
    }
}

/// hermes `_extract_text`: text.content → richText → audio recognition →
/// file name → doc card.
pub fn extract_text(payload: &Value) -> String {
    if let Some(content) = payload.pointer("/text/content").and_then(|v| v.as_str()) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(rich_list) = payload
        .pointer("/richTextContent/richTextList")
        .or_else(|| payload.pointer("/richText"))
        .and_then(|v| v.as_array())
    {
        let parts: Vec<String> = rich_list
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .or_else(|| item.get("content"))
                    .and_then(|v| v.as_str())
            })
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .collect();
        if !parts.is_empty() {
            return parts.join(" ");
        }
    }
    let msg_type = payload
        .get("msgtype")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match msg_type {
        "audio" => {
            if let Some(recognition) = payload
                .pointer("/extensions/content/recognition")
                .and_then(|v| v.as_str())
            {
                if !recognition.trim().is_empty() {
                    return recognition.trim().to_string();
                }
            }
        }
        "file" => {
            if let Some(fname) = payload
                .pointer("/extensions/content/fileName")
                .and_then(|v| v.as_str())
            {
                if !fname.is_empty() {
                    return format!("[文件] {fname}");
                }
            }
        }
        "card" => {
            let title = payload
                .pointer("/extensions/card/title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut doc_url = String::new();
            if let Some(raw_content) = payload.pointer("/extensions/card/content") {
                if let Some(obj) = raw_content.as_object() {
                    doc_url = obj
                        .get("url")
                        .or_else(|| obj.get("docUrl"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                } else if let Some(s) = raw_content.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        doc_url = serde_json::from_str::<Value>(trimmed)
                            .ok()
                            .and_then(|parsed| {
                                parsed
                                    .get("url")
                                    .or_else(|| parsed.get("docUrl"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                            })
                            .unwrap_or_else(|| trimmed.to_string());
                    }
                }
            }
            let mut parts = Vec::new();
            if !title.is_empty() {
                parts.push(format!("[文档] {title}"));
            }
            if !doc_url.is_empty() {
                parts.push(doc_url);
            }
            if !parts.is_empty() {
                return parts.join(" ");
            }
        }
        _ => {}
    }
    String::new()
}

/// Collect `(downloadCode, filename)` media refs (hermes `_extract_media`
/// core) and resolve them through the robot file-download API.
async fn collect_media_refs(
    runtime: &Arc<Runtime>,
    payload: &Value,
    robot_code: &str,
) -> Vec<Option<MediaAttachment>> {
    let mut refs: Vec<(String, String)> = Vec::new();
    let msg_type = payload
        .get("msgtype")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if let Some(rich_list) = payload
        .pointer("/richTextContent/richTextList")
        .or_else(|| payload.pointer("/richText"))
        .and_then(|v| v.as_array())
    {
        for item in rich_list {
            if let Some(code) = item
                .get("downloadCode")
                .or_else(|| item.get("download_code"))
                .and_then(|v| v.as_str())
            {
                if !code.is_empty() {
                    let ext = item
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("file");
                    refs.push((code.to_string(), format!("rich_{ext}")));
                }
            }
        }
    }
    if matches!(msg_type, "file" | "image") {
        if let Some(code) = payload
            .pointer("/extensions/content/downloadCode")
            .and_then(|v| v.as_str())
        {
            if !code.is_empty() {
                let fname = payload
                    .pointer("/extensions/content/fileName")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        format!("attachment.{}", if msg_type == "image" { "jpg" } else { "bin" })
                    });
                refs.push((code.to_string(), fname));
            }
        }
    }

    let mut results = Vec::new();
    for (code, fname) in refs {
        results.push(runtime.download_media_code(&code, robot_code, &fname).await);
    }
    results
}

struct DingTalkSender {
    runtime: Arc<Runtime>,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for DingTalkSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        if let Err(e) = self.runtime.send_markdown(chat_id, text).await {
            eprintln!("[dingtalk] send_text to {chat_id} failed: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_url_validation() {
        assert!(is_dingtalk_webhook_url(
            "https://oapi.dingtalk.com/robot/sendBySession?session=x"
        ));
        assert!(is_dingtalk_webhook_url("https://api.dingtalk.com/v1.0/x"));
        assert!(!is_dingtalk_webhook_url("https://evil.com/oapi.dingtalk.com"));
        assert!(!is_dingtalk_webhook_url("http://oapi.dingtalk.com/x"));
    }

    #[test]
    fn normalize_markdown_numbered_lists() {
        let input = "Intro\n1. first\n2. second";
        let out = normalize_markdown(input);
        assert!(out.contains("Intro\n\n1. first"));
    }

    #[test]
    fn normalize_markdown_dedents_fences() {
        let input = "text\n    ```rust\n    code\n    ```";
        let out = normalize_markdown(input);
        assert!(out.contains("\n```rust"));
    }

    #[test]
    fn extract_text_plain() {
        let payload = json!({"text": {"content": "  hello bot  "}, "msgtype": "text"});
        assert_eq!(extract_text(&payload), "hello bot");
    }

    #[test]
    fn extract_text_audio_recognition() {
        let payload = json!({
            "msgtype": "audio",
            "extensions": {"content": {"recognition": "语音内容"}},
        });
        assert_eq!(extract_text(&payload), "语音内容");
    }

    #[test]
    fn extract_text_file_name() {
        let payload = json!({
            "msgtype": "file",
            "extensions": {"content": {"fileName": "report.pdf"}},
        });
        assert_eq!(extract_text(&payload), "[文件] report.pdf");
    }

    #[test]
    fn extract_text_card_doc() {
        let payload = json!({
            "msgtype": "card",
            "extensions": {"card": {"title": "设计文档", "content": "{\"url\": \"https://docs.example.com/d/1\"}"}},
        });
        let text = extract_text(&payload);
        assert!(text.contains("[文档] 设计文档"));
        assert!(text.contains("https://docs.example.com/d/1"));
    }

    #[test]
    fn group_gating_rules() {
        let cfg = ResolvedDingTalk {
            client_id: String::new(),
            client_secret: String::new(),
            require_mention: true,
            free_response_chats: vec!["free-chat".into()],
            allowed_chats: vec!["allowed-chat".into(), "free-chat".into()],
            allowed_users: Vec::new(),
            mention_patterns: vec!["^小马".into()],
        };
        let runtime = Runtime {
            cfg,
            client: reqwest::Client::new(),
            session_webhooks: Mutex::new(HashMap::new()),
            dedup: Mutex::new(HashMap::new()),
            access_token: Mutex::new((String::new(), std::time::Instant::now())),
            mention_regexes: vec![regex::Regex::new("^小马").unwrap()],
        };
        // allowed_chats hard gate
        assert!(!runtime.should_process("hi", true, "other-chat", true));
        // mention passes
        assert!(runtime.should_process("hi", true, "allowed-chat", true));
        // no mention, not free → blocked
        assert!(!runtime.should_process("hi", true, "allowed-chat", false));
        // wake word passes
        assert!(runtime.should_process("小马你好", true, "allowed-chat", false));
        // free chat passes without mention
        assert!(runtime.should_process("hi", true, "free-chat", false));
        // DMs always pass
        assert!(runtime.should_process("hi", false, "any", false));
    }

    #[test]
    fn user_allowlist_matching() {
        let runtime = Runtime {
            cfg: ResolvedDingTalk {
                client_id: String::new(),
                client_secret: String::new(),
                require_mention: false,
                free_response_chats: Vec::new(),
                allowed_chats: Vec::new(),
                allowed_users: vec!["Manager123".into()],
                mention_patterns: Vec::new(),
            },
            client: reqwest::Client::new(),
            session_webhooks: Mutex::new(HashMap::new()),
            dedup: Mutex::new(HashMap::new()),
            access_token: Mutex::new((String::new(), std::time::Instant::now())),
            mention_regexes: Vec::new(),
        };
        assert!(runtime.is_user_allowed("x", "manager123"));
        assert!(runtime.is_user_allowed("MANAGER123", ""));
        assert!(!runtime.is_user_allowed("someone", "else"));
    }

    #[test]
    fn webhook_expiry_margin() {
        let mut map = HashMap::new();
        let future_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64)
            + 60 * 60 * 1000;
        map.insert("chat1".to_string(), ("https://oapi.dingtalk.com/x".to_string(), future_ms));
        assert!(Runtime::valid_webhook(&map, "chat1").is_some());
        // Expires within the 5-minute margin → rejected.
        let soon_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64)
            + 60 * 1000;
        map.insert("chat2".to_string(), ("https://oapi.dingtalk.com/y".to_string(), soon_ms));
        assert!(Runtime::valid_webhook(&map, "chat2").is_none());
        // No expiry recorded → always valid.
        map.insert("chat3".to_string(), ("https://oapi.dingtalk.com/z".to_string(), 0));
        assert!(Runtime::valid_webhook(&map, "chat3").is_some());
    }

    #[tokio::test]
    async fn dedup_behavior() {
        let runtime = Runtime {
            cfg: ResolvedDingTalk {
                client_id: String::new(),
                client_secret: String::new(),
                require_mention: false,
                free_response_chats: Vec::new(),
                allowed_chats: Vec::new(),
                allowed_users: Vec::new(),
                mention_patterns: Vec::new(),
            },
            client: reqwest::Client::new(),
            session_webhooks: Mutex::new(HashMap::new()),
            dedup: Mutex::new(HashMap::new()),
            access_token: Mutex::new((String::new(), std::time::Instant::now())),
            mention_regexes: Vec::new(),
        };
        assert!(!runtime.is_duplicate("m1").await);
        assert!(runtime.is_duplicate("m1").await);
    }
}
