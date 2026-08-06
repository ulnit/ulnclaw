//! LINE Messaging API platform adapter — port of hermes
//! `plugins/platforms/line` @ v2026.8.3 (adapter.py).
//!
//! Webhook ingress (gateway-mounted `/webhooks/line`; hermes runs a
//! dedicated aiohttp server on `/line/webhook`): raw-body
//! `X-Line-Signature` verification (base64 HMAC-SHA256 keyed by the
//! channel secret, constant-time compare), 1 MiB body cap,
//! webhook-event-id dedup (LRU-1000 against at-least-once retries).
//!
//! Intake resolves the event source (user/group/room → chat id) and
//! applies hermes' three-allowlist gate (`LINE_ALLOWED_USERS` U-ids,
//! `LINE_ALLOWED_GROUPS` C-ids, `LINE_ALLOWED_ROOMS` R-ids,
//! `LINE_ALLOW_ALL_USERS` dev escape hatch) unioned with pairing.
//! Text messages dispatch directly; image/video/audio/file messages
//! download via `api-data.line.me/v2/bot/message/<id>/content` into
//! the media cache; stickers/locations degrade to text notes.
//!
//! Replies prefer the single-use reply token (free, ~60 s TTL) and
//! fall back to the metered Push API when the token is rejected.
//! Messages are markdown-stripped with URLs preserved (`[label](url)`
//! → `label (url)`) and smart-chunked at 4500 chars with a 5-message
//! per-call budget (hermes `split_for_line`). Outbound media needs
//! public HTTPS URLs in LINE's API and is not ported (documented
//! divergence); the slow-LLM postback-button state machine is not
//! ported either.

use crate::messaging::{Dispatcher, MediaAttachment, MessageEvent};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// hermes `LINE_SAFE_BUBBLE_CHARS` (LINE bubble limit is 5000).
const LINE_SAFE_BUBBLE_CHARS: usize = 4500;
/// hermes `LINE_MAX_MESSAGES_PER_CALL`.
const LINE_MAX_MESSAGES_PER_CALL: usize = 5;
/// hermes `WEBHOOK_BODY_MAX_BYTES`.
const WEBHOOK_BODY_MAX_BYTES: usize = 1_048_576;
const EVENT_DEDUP_MAX: usize = 1000;
const API_TIMEOUT: Duration = Duration::from_secs(30);
const LINE_REPLY_URL: &str = "https://api.line.me/v2/bot/message/reply";
const LINE_PUSH_URL: &str = "https://api.line.me/v2/bot/message/push";
const LINE_LOADING_URL: &str = "https://api.line.me/v2/bot/chat/loading/start";
const LINE_CONTENT_URL: &str = "https://api-data.line.me/v2/bot/message";

/// `[messaging.line]` — LINE adapter (hermes `platforms.line` plugin
/// config + `LINE_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LineConfig {
    pub enabled: bool,
    /// Channel access token (fallback `LINE_CHANNEL_ACCESS_TOKEN`).
    pub channel_access_token: String,
    /// Channel secret for webhook signatures (fallback
    /// `LINE_CHANNEL_SECRET`).
    pub channel_secret: String,
    /// U-prefixed user ids (fallback `LINE_ALLOWED_USERS`).
    pub allowed_users: Vec<String>,
    /// C-prefixed group ids (fallback `LINE_ALLOWED_GROUPS`).
    pub allowed_groups: Vec<String>,
    /// R-prefixed room ids (fallback `LINE_ALLOWED_ROOMS`).
    pub allowed_rooms: Vec<String>,
    /// Dev escape hatch (fallback `LINE_ALLOW_ALL_USERS`).
    pub allow_all_users: bool,
    /// Cron/notification delivery chat (fallback `LINE_HOME_CHANNEL`).
    pub home_channel: String,
}

impl Default for LineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channel_access_token: String::new(),
            channel_secret: String::new(),
            allowed_users: Vec::new(),
            allowed_groups: Vec::new(),
            allowed_rooms: Vec::new(),
            allow_all_users: false,
            home_channel: String::new(),
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
pub struct ResolvedLine {
    pub channel_access_token: String,
    pub channel_secret: String,
    pub allowed_users: Vec<String>,
    pub allowed_groups: Vec<String>,
    pub allowed_rooms: Vec<String>,
    pub allow_all_users: bool,
    pub home_channel: String,
}

impl LineConfig {
    pub fn resolve(&self) -> ResolvedLine {
        ResolvedLine {
            channel_access_token: env_trim("LINE_CHANNEL_ACCESS_TOKEN")
                .unwrap_or_else(|| self.channel_access_token.clone()),
            channel_secret: env_trim("LINE_CHANNEL_SECRET")
                .unwrap_or_else(|| self.channel_secret.clone()),
            allowed_users: env_list("LINE_ALLOWED_USERS")
                .unwrap_or_else(|| self.allowed_users.clone()),
            allowed_groups: env_list("LINE_ALLOWED_GROUPS")
                .unwrap_or_else(|| self.allowed_groups.clone()),
            allowed_rooms: env_list("LINE_ALLOWED_ROOMS")
                .unwrap_or_else(|| self.allowed_rooms.clone()),
            allow_all_users: env_trim("LINE_ALLOW_ALL_USERS")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(self.allow_all_users),
            home_channel: env_trim("LINE_HOME_CHANNEL")
                .unwrap_or_else(|| self.home_channel.clone()),
        }
    }
}

/// hermes `verify_line_signature` — base64 HMAC-SHA256 over the raw
/// body, constant-time compare.
pub fn verify_line_signature(body: &[u8], signature: &str, channel_secret: &str) -> bool {
    if signature.is_empty() || channel_secret.is_empty() {
        return false;
    }
    let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(channel_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// hermes `_resolve_chat` — source block → (chat_id, chat_type).
pub fn resolve_chat(source: &Value) -> (String, &'static str) {
    let src_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match src_type {
        "group" => (
            source
                .get("groupId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "group",
        ),
        "room" => (
            source
                .get("roomId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "room",
        ),
        "user" => (
            source
                .get("userId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "dm",
        ),
        _ => (String::new(), "dm"),
    }
}

/// hermes `_allowed_for_source` — three-list gate.
pub fn allowed_for_source(cfg: &ResolvedLine, source: &Value) -> bool {
    if cfg.allow_all_users {
        return true;
    }
    let src_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match src_type {
        "user" => {
            let uid = source.get("userId").and_then(|v| v.as_str()).unwrap_or("");
            !uid.is_empty() && cfg.allowed_users.iter().any(|u| u == uid || u == "*")
        }
        "group" => {
            let gid = source.get("groupId").and_then(|v| v.as_str()).unwrap_or("");
            !gid.is_empty() && cfg.allowed_groups.iter().any(|g| g == gid || g == "*")
        }
        "room" => {
            let rid = source.get("roomId").and_then(|v| v.as_str()).unwrap_or("");
            !rid.is_empty() && cfg.allowed_rooms.iter().any(|r| r == rid || r == "*")
        }
        _ => false,
    }
}

/// hermes `strip_markdown_preserving_urls` — LINE renders markdown
/// literally but auto-links bare URLs.
pub fn strip_markdown_preserving_urls(text: &str) -> String {
    let mut out = text.to_string();
    // Code fences: keep content, drop the fence lines.
    let mut unfenced = String::new();
    let mut in_fence = false;
    for line in out.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        unfenced.push_str(line);
        unfenced.push('\n');
    }
    out = unfenced.trim_end_matches('\n').to_string();
    // Inline code.
    out = crate::sms::strip_paired_pub(&out, "`");
    // Markdown links → "label (url)" (http(s) targets only, hermes).
    out = links_to_label_url(&out);
    // Bold then italic.
    out = crate::sms::strip_paired_pub(&out, "**");
    out = strip_single_star_italics(&out);
    // Headings and bullets.
    out = out
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes)
                && trimmed.chars().nth(hashes).map(|c| c.is_whitespace()) == Some(true)
            {
                trimmed[hashes..].trim_start()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out = out
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ")
            {
                format!("{}• {}", &line[..line.len() - trimmed.len()], &trimmed[2..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out
}

fn links_to_label_url(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('[') {
        let Some(close_label) = rest[start..].find(']') else {
            break;
        };
        let label_end = start + close_label;
        if rest[label_end..].starts_with("](http://") || rest[label_end..].starts_with("](https://")
        {
            let Some(close_target) = rest[label_end..].find(')') else {
                break;
            };
            let label = &rest[start + 1..label_end];
            let target = &rest[label_end + 2..label_end + close_target];
            if target.contains(char::is_whitespace) {
                out.push_str(&rest[..label_end + close_target + 1]);
                rest = &rest[label_end + close_target + 1..];
                continue;
            }
            out.push_str(&rest[..start]);
            out.push_str(&format!("{label} ({target})"));
            rest = &rest[label_end + close_target + 1..];
        } else {
            out.push_str(&rest[..label_end + 1]);
            rest = &rest[label_end + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// hermes `_MD_ITAL_RE` approximation: paired `*text*` that is not
/// `**` and has no whitespace at the inner edges.
fn strip_single_star_italics(input: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*'
            && (i == 0 || chars[i - 1] != '*')
            && i + 1 < chars.len()
            && chars[i + 1] != '*'
            && !chars[i + 1].is_whitespace()
        {
            // Find closing single star.
            let mut j = i + 1;
            let mut found = None;
            while j < chars.len() {
                if chars[j] == '*'
                    && (j + 1 >= chars.len() || chars[j + 1] != '*')
                    && (j == i + 1 || !chars[j - 1].is_whitespace())
                {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                out.extend(&chars[i + 1..end]);
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// hermes `split_for_line` — paragraph/line/space-aware chunking with
/// a 5-chunk budget and ellipsis truncation.
pub fn split_for_line(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut remaining = text.to_string();
    while !remaining.is_empty() && chunks.len() < LINE_MAX_MESSAGES_PER_CALL {
        if remaining.chars().count() <= max_chars {
            chunks.push(remaining);
            remaining = String::new();
            break;
        }
        let window: String = remaining.chars().take(max_chars).collect();
        let cut = window
            .rfind("\n\n")
            .filter(|c| *c >= max_chars / 2)
            .or_else(|| window.rfind('\n').filter(|c| *c >= max_chars / 2))
            .or_else(|| window.rfind(' ').filter(|c| *c >= max_chars / 2))
            .unwrap_or(max_chars);
        let (head, tail) = remaining.split_at(
            remaining
                .char_indices()
                .nth(cut)
                .map(|(idx, _)| idx)
                .unwrap_or(remaining.len()),
        );
        chunks.push(head.trim_end().to_string());
        remaining = tail.trim_start().to_string();
    }
    if !remaining.is_empty() {
        if let Some(last) = chunks.last_mut() {
            let truncated: String = last.chars().take(max_chars - 1).collect();
            *last = format!("{}…", truncated.trim_end());
        } else {
            let truncated: String = remaining.chars().take(max_chars - 1).collect();
            chunks.push(format!("{truncated}…"));
        }
    }
    chunks
}

struct Runtime {
    cfg: ResolvedLine,
    client: reqwest::Client,
    /// webhookEventId dedup (hermes `_EventIdDedup` LRU-1000).
    seen_events: Mutex<Vec<String>>,
}

static RUNTIME: std::sync::OnceLock<Arc<Runtime>> = std::sync::OnceLock::new();

/// Register the adapter (called from `run_messaging` when enabled).
pub fn register(cfg: &LineConfig) {
    let resolved = cfg.resolve();
    if resolved.channel_access_token.is_empty() || resolved.channel_secret.is_empty() {
        eprintln!(
            "[line] enabled but LINE_CHANNEL_ACCESS_TOKEN/LINE_CHANNEL_SECRET are not both set — webhook route will reject"
        );
    }
    let runtime = Arc::new(Runtime {
        client: reqwest::Client::builder()
            .timeout(API_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        cfg: resolved,
        seen_events: Mutex::new(Vec::new()),
    });
    let _ = RUNTIME.set(runtime.clone());
    crate::messaging::register_platform_sender(
        "line",
        Arc::new(LineSender {
            runtime: runtime.clone(),
        }),
    );
}

fn runtime() -> Option<Arc<Runtime>> {
    RUNTIME.get().cloned()
}

/// Webhook response handed back to the gateway route.
pub struct LineWebhookResponse {
    pub status: u16,
    pub body: Value,
}

fn ack() -> LineWebhookResponse {
    LineWebhookResponse {
        status: 200,
        body: json!({}),
    }
}

/// Gateway webhook entry point, mounted at `/webhooks/line`.
pub async fn line_handle_webhook(
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    body: &[u8],
    headers: &[(String, String)],
) -> LineWebhookResponse {
    if body.len() > WEBHOOK_BODY_MAX_BYTES {
        return LineWebhookResponse {
            status: 413,
            body: json!({ "error": "body too large" }),
        };
    }
    let Some(runtime) = runtime() else {
        return LineWebhookResponse {
            status: 503,
            body: json!({ "error": "line adapter not registered" }),
        };
    };
    let signature = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-line-signature"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    if !verify_line_signature(body, &signature, &runtime.cfg.channel_secret) {
        return LineWebhookResponse {
            status: 401,
            body: json!({ "error": "invalid signature" }),
        };
    }
    let payload: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            return LineWebhookResponse {
                status: 400,
                body: json!({ "error": "invalid JSON" }),
            }
        }
    };
    let events = payload
        .get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for event in events {
        handle_event(&runtime, dispatcher, pairing, &event).await;
    }
    ack()
}

async fn remember_event(runtime: &Runtime, event_id: &str) -> bool {
    if event_id.is_empty() {
        return true;
    }
    let mut seen = runtime.seen_events.lock().await;
    if seen.iter().any(|e| e == event_id) {
        return false;
    }
    seen.push(event_id.to_string());
    if seen.len() > EVENT_DEDUP_MAX {
        let drain = seen.len() - EVENT_DEDUP_MAX;
        seen.drain(..drain);
    }
    true
}

async fn handle_event(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: Option<&crate::pairing::PairingStore>,
    event: &Value,
) {
    if event.get("type").and_then(|v| v.as_str()) != Some("message") {
        return;
    }
    let event_id = event
        .get("webhookEventId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !remember_event(runtime, event_id).await {
        return;
    }
    let source = event.get("source").cloned().unwrap_or(json!({}));
    let (chat_id, chat_type) = resolve_chat(&source);
    if chat_id.is_empty() {
        return;
    }
    // Three-list gate ∪ pairing.
    if !allowed_for_source(&runtime.cfg, &source) {
        let sender_id = source
            .get("userId")
            .and_then(|v| v.as_str())
            .unwrap_or(&chat_id)
            .to_string();
        if let Some(store) = pairing {
            if !store.is_approved("line", &sender_id) {
                if let Some(code_msg) =
                    crate::messaging::pairing_offer_public(store, "line", &sender_id, &sender_id)
                {
                    let _ = push_text(runtime, &chat_id, &code_msg).await;
                }
            }
        } else {
            eprintln!("[line] unauthorized sender {sender_id} — add to the allowed lists");
        }
        return;
    }
    let reply_token = event
        .get("replyToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message = event.get("message").cloned().unwrap_or(json!({}));
    let msg_type = message
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message_id = message
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut text = String::new();
    let mut attachments = Vec::new();
    match msg_type {
        "text" => {
            text = message
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
        }
        "image" | "video" | "audio" | "file" => {
            match fetch_content(runtime, &message_id).await {
                Ok((data, content_type)) => {
                    let name = message
                        .get("fileName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mime: String = if content_type.is_empty() {
                        guess_mime(msg_type).to_string()
                    } else {
                        content_type
                    };
                    match crate::media_cache::cache_media_bytes(
                        &crate::config::ulnclaw_home(),
                        &data,
                        &mime,
                        name,
                    ) {
                        Ok(path) => attachments.push(MediaAttachment {
                            path,
                            mime: mime.to_string(),
                            bytes: data.len() as u64,
                            original_name: name.to_string(),
                        }),
                        Err(e) => eprintln!("[line] media cache failed: {e}"),
                    }
                }
                Err(e) => eprintln!("[line] content fetch failed: {e}"),
            }
            if let Some(caption) = message.get("text").and_then(|v| v.as_str()) {
                text = caption.trim().to_string();
            }
        }
        "sticker" => text = "[sticker]".to_string(),
        "location" => {
            let lat = message.get("latitude").map(|v| v.to_string()).unwrap_or_default();
            let lng = message.get("longitude").map(|v| v.to_string()).unwrap_or_default();
            text = format!("[location: {lat}, {lng}]");
        }
        _ => return,
    }
    if text.is_empty() && attachments.is_empty() {
        return;
    }
    let sender_id = source
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or(&chat_id)
        .to_string();
    // Best-effort loading indicator while the agent thinks.
    send_loading(runtime, &chat_id).await;
    let mut gate_event = MessageEvent {
        platform: "line".into(),
        chat_id: chat_id.clone(),
        sender_name: sender_id.clone(),
        sender_id,
        text,
        message_id,
        attachments,
    };
    if !crate::messaging::pre_gateway_dispatch_gate_public(&mut gate_event).await {
        return;
    }
    let outcome = match dispatcher.handle_event(gate_event).await {
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
    if reply_text.is_empty() {
        return;
    }
    send_reply(runtime, &chat_id, &reply_token, &reply_text).await;
    let _ = chat_type;
}

/// Reply-token-first delivery with Push fallback (hermes core flow).
async fn send_reply(runtime: &Runtime, chat_id: &str, reply_token: &str, content: &str) {
    let formatted = strip_markdown_preserving_urls(content);
    let chunks = split_for_line(&formatted, LINE_SAFE_BUBBLE_CHARS);
    if chunks.is_empty() {
        return;
    }
    for batch in chunks.chunks(LINE_MAX_MESSAGES_PER_CALL) {
        let messages: Vec<Value> = batch
            .iter()
            .map(|t| json!({ "type": "text", "text": t }))
            .collect();
        if !reply_token.is_empty() {
            let ok = runtime
                .client
                .post(LINE_REPLY_URL)
                .bearer_auth(&runtime.cfg.channel_access_token)
                .json(&json!({ "replyToken": reply_token, "messages": messages }))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ok {
                continue;
            }
            eprintln!("[line] reply token rejected — falling back to push");
        }
        if let Err(e) = push_messages(runtime, chat_id, &messages).await {
            eprintln!("[line] push to {chat_id} failed: {e}");
        }
    }
}

async fn push_messages(runtime: &Runtime, chat_id: &str, messages: &[Value]) -> Result<(), String> {
    let resp = runtime
        .client
        .post(LINE_PUSH_URL)
        .bearer_auth(&runtime.cfg.channel_access_token)
        .json(&json!({ "to": chat_id, "messages": messages }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("HTTP {status}: {}", &body[..body.len().min(200)]))
    }
}

async fn push_text(runtime: &Runtime, chat_id: &str, text: &str) -> Result<(), String> {
    let messages: Vec<Value> = split_for_line(text, LINE_SAFE_BUBBLE_CHARS)
        .into_iter()
        .map(|t| json!({ "type": "text", "text": t }))
        .collect();
    push_messages(runtime, chat_id, &messages).await
}

async fn send_loading(runtime: &Runtime, chat_id: &str) {
    let _ = runtime
        .client
        .post(LINE_LOADING_URL)
        .bearer_auth(&runtime.cfg.channel_access_token)
        .json(&json!({ "chatId": chat_id, "loadingSeconds": 60 }))
        .send()
        .await;
}

async fn fetch_content(runtime: &Runtime, message_id: &str) -> Result<(Vec<u8>, String), String> {
    let url = format!("{LINE_CONTENT_URL}/{message_id}/content");
    let resp = runtime
        .client
        .get(&url)
        .bearer_auth(&runtime.cfg.channel_access_token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("content HTTP {}", resp.status()));
    }
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    Ok((bytes.to_vec(), content_type))
}

fn guess_mime(msg_type: &str) -> &'static str {
    match msg_type {
        "image" => "image/jpeg",
        "video" => "video/mp4",
        "audio" => "audio/mp4",
        _ => "application/octet-stream",
    }
}

struct LineSender {
    runtime: Arc<Runtime>,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for LineSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        if let Err(e) = push_text(&self.runtime, chat_id, text).await {
            eprintln!("[line] send_text to {chat_id} failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig_for(body: &[u8], secret: &str) -> String {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    #[test]
    fn signature_roundtrip() {
        let body = br#"{"events":[]}"#;
        let sig = sig_for(body, "secret123");
        assert!(verify_line_signature(body, &sig, "secret123"));
        assert!(!verify_line_signature(body, &sig, "wrong"));
        assert!(!verify_line_signature(body, "bogus", "secret123"));
        assert!(!verify_line_signature(body, "", "secret123"));
        assert!(!verify_line_signature(body, &sig, ""));
    }

    #[test]
    fn resolve_chat_sources() {
        let (id, t) = resolve_chat(&json!({"type":"user","userId":"U123"}));
        assert_eq!((id.as_str(), t), ("U123", "dm"));
        let (id, t) = resolve_chat(&json!({"type":"group","groupId":"C456","userId":"U1"}));
        assert_eq!((id.as_str(), t), ("C456", "group"));
        let (id, t) = resolve_chat(&json!({"type":"room","roomId":"R789"}));
        assert_eq!((id.as_str(), t), ("R789", "room"));
        let (id, _) = resolve_chat(&json!({}));
        assert!(id.is_empty());
    }

    #[test]
    fn three_list_gate() {
        let cfg = ResolvedLine {
            channel_access_token: String::new(),
            channel_secret: String::new(),
            allowed_users: vec!["U1".into()],
            allowed_groups: vec!["C1".into()],
            allowed_rooms: Vec::new(),
            allow_all_users: false,
            home_channel: String::new(),
        };
        assert!(allowed_for_source(&cfg, &json!({"type":"user","userId":"U1"})));
        assert!(!allowed_for_source(&cfg, &json!({"type":"user","userId":"U2"})));
        assert!(allowed_for_source(&cfg, &json!({"type":"group","groupId":"C1"})));
        assert!(!allowed_for_source(&cfg, &json!({"type":"room","roomId":"R1"})));
        let mut open = cfg.clone();
        open.allow_all_users = true;
        assert!(allowed_for_source(&open, &json!({"type":"room","roomId":"R1"})));
    }

    #[test]
    fn markdown_stripping_preserves_urls() {
        let out = strip_markdown_preserving_urls("**bold** and `code`");
        assert_eq!(out, "bold and code");
        let out = strip_markdown_preserving_urls("[docs](https://example.com/a)");
        assert_eq!(out, "docs (https://example.com/a)");
        let out = strip_markdown_preserving_urls("## Title\n- item");
        assert_eq!(out, "Title\n• item");
        let out = strip_markdown_preserving_urls("```\ncode line\n```");
        assert_eq!(out, "code line");
        // Bare URLs stay untouched (LINE auto-links them).
        let out = strip_markdown_preserving_urls("see https://example.com now");
        assert_eq!(out, "see https://example.com now");
    }

    #[test]
    fn split_respects_budget_and_ellipsis() {
        // 30000 chars > 5 x 4500 budget → final chunk truncated with an
        // ellipsis (hermes split_for_line semantics).
        let text = "word ".repeat(6000);
        let chunks = split_for_line(text.trim(), LINE_SAFE_BUBBLE_CHARS);
        assert!(chunks.len() <= LINE_MAX_MESSAGES_PER_CALL);
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(chunk.chars().count() <= LINE_SAFE_BUBBLE_CHARS);
        }
        assert!(chunks.last().unwrap().ends_with('…'));
        // Short text passes through.
        assert_eq!(split_for_line("hi", LINE_SAFE_BUBBLE_CHARS), vec!["hi"]);
    }

    #[test]
    fn split_prefers_paragraph_breaks() {
        let para = "a".repeat(100);
        let text = format!("{para}\n\n{para}\n\n{para}");
        let chunks = split_for_line(&text, 220);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].chars().count() <= 220);
    }

    #[test]
    fn resolve_env_precedence() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::set_var("LINE_CHANNEL_SECRET", "env-secret");
        std::env::set_var("LINE_ALLOWED_GROUPS", "C1, C2");
        let cfg = LineConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(resolved.channel_secret, "env-secret");
        assert_eq!(
            resolved.allowed_groups,
            vec!["C1".to_string(), "C2".to_string()]
        );
        std::env::remove_var("LINE_CHANNEL_SECRET");
        std::env::remove_var("LINE_ALLOWED_GROUPS");
    }

    #[test]
    fn webhook_body_limit_matches_hermes() {
        assert_eq!(WEBHOOK_BODY_MAX_BYTES, 1_048_576);
        assert_eq!(LINE_SAFE_BUBBLE_CHARS, 4500);
        assert_eq!(LINE_MAX_MESSAGES_PER_CALL, 5);
    }
}
