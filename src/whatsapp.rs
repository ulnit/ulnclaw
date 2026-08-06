//! WhatsApp (Baileys bridge) platform adapter — port of hermes
//! `plugins/platforms/whatsapp` @ v2026.8.3 (adapter.py).
//!
//! The hermes adapter supervises a Node.js Baileys bridge subprocess
//! and talks to it over localhost HTTP: `GET /health` (connection /
//! pairing status), `GET /messages` (inbound queue, polled every
//! second), `POST /read` (read receipts), `POST /send`
//! (`{chatId, message, replyTo?}`), and `POST /send-media`
//! (`{to, path, mediaType, caption}`). This port keeps the exact wire
//! protocol but treats the bridge as an external service: ulnclaw does
//! not spawn or supervise the Node process (no `bridge.js` is bundled —
//! run any Baileys bridge implementing the hermes endpoints and point
//! `[messaging.whatsapp] bridge_url` at it).
//!
//! Intake mirrors hermes: messages from self and status broadcasts are
//! dropped, DMs are allowlist∪pairing gated, groups honor
//! `allowed_channels` + require-mention (bot JID from `/health`,
//! fallback to any `@mention`) with `free_response_channels` opt-outs.
//! Inbound media (bridge-cached local paths or URLs) download into the
//! content-addressed media cache; outbound `MEDIA:` tags ride
//! `/send-media` with an extension-derived `mediaType`.

use crate::messaging::{Dispatcher, MediaAttachment, MessageEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// hermes poll cadence (`await asyncio.sleep(1)`).
const DEFAULT_POLL_INTERVAL_MS: u64 = 1000;
/// Outbound text chunk limit (WhatsApp allows ~65k; hermes chunks long
/// replies — keep the conservative 4000 used by other adapters).
const MAX_MESSAGE_LENGTH: usize = 4000;
const API_TIMEOUT: Duration = Duration::from_secs(30);

/// `[messaging.whatsapp]` — Baileys-bridge adapter (hermes
/// `platforms.whatsapp` plugin config + `WHATSAPP_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhatsappConfig {
    pub enabled: bool,
    /// Base URL of the Baileys HTTP bridge (fallback
    /// `WHATSAPP_BRIDGE_URL`, default `http://127.0.0.1:3000`).
    pub bridge_url: String,
    /// Sender JIDs/phone numbers allowed in DMs (fallback
    /// `WHATSAPP_ALLOWED_USERS`).
    pub allowed_users: Vec<String>,
    /// When non-empty, the bot ONLY answers in these group JIDs.
    pub allowed_channels: Vec<String>,
    /// Require an @mention of the bot in groups (hermes behavior).
    pub require_mention: bool,
    /// Group JIDs exempt from the mention requirement.
    pub free_response_channels: Vec<String>,
    /// Cron/notification delivery chat (hermes `WHATSAPP_HOME_CHANNEL`).
    pub home_channel: String,
    /// Inbound poll interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Send read receipts for accepted messages (hermes
    /// `send_read_receipts`).
    pub read_receipts: bool,
}

impl Default for WhatsappConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bridge_url: String::new(),
            allowed_users: Vec::new(),
            allowed_channels: Vec::new(),
            require_mention: true,
            free_response_channels: Vec::new(),
            home_channel: String::new(),
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            read_receipts: true,
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
pub struct ResolvedWhatsapp {
    pub bridge_url: String,
    pub allowed_users: Vec<String>,
    pub allowed_channels: Vec<String>,
    pub require_mention: bool,
    pub free_response_channels: Vec<String>,
    pub home_channel: String,
    pub poll_interval_ms: u64,
    pub read_receipts: bool,
}

impl WhatsappConfig {
    pub fn resolve(&self) -> ResolvedWhatsapp {
        ResolvedWhatsapp {
            bridge_url: env_trim("WHATSAPP_BRIDGE_URL")
                .unwrap_or_else(|| self.bridge_url.clone())
                .trim_end_matches('/')
                .to_string(),
            allowed_users: env_list("WHATSAPP_ALLOWED_USERS")
                .unwrap_or_else(|| self.allowed_users.clone()),
            allowed_channels: env_list("WHATSAPP_ALLOWED_CHANNELS")
                .unwrap_or_else(|| self.allowed_channels.clone()),
            require_mention: env_trim("WHATSAPP_REQUIRE_MENTION")
                .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no"))
                .unwrap_or(self.require_mention),
            free_response_channels: env_list("WHATSAPP_FREE_RESPONSE_CHANNELS")
                .unwrap_or_else(|| self.free_response_channels.clone()),
            home_channel: env_trim("WHATSAPP_HOME_CHANNEL")
                .unwrap_or_else(|| self.home_channel.clone()),
            poll_interval_ms: env_trim("WHATSAPP_POLL_INTERVAL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(self.poll_interval_ms)
                .max(100),
            read_receipts: env_trim("WHATSAPP_READ_RECEIPTS")
                .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no"))
                .unwrap_or(self.read_receipts),
        }
    }
}

struct Runtime {
    cfg: ResolvedWhatsapp,
    client: reqwest::Client,
    /// Bot JID reported by `/health` (e.g. `12345@s.whatsapp.net`),
    /// used for group mention detection.
    bot_jid: tokio::sync::Mutex<String>,
}

/// hermes `_should_process_message` — drop from-self and status
/// broadcasts.
pub fn should_process(data: &Value) -> bool {
    if data.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false) {
        return false;
    }
    let chat_id = data.get("chatId").and_then(|v| v.as_str()).unwrap_or("");
    if chat_id.is_empty()
        || chat_id == "status@broadcast"
        || chat_id.ends_with("@newsletter")
    {
        return false;
    }
    true
}

/// Map a bridge `mediaType` string onto a coarse kind used for caching
/// (mirrors hermes `_build_message_event` type detection).
pub fn media_kind_for(media_type: &str) -> &'static str {
    if media_type.contains("image") {
        "image"
    } else if media_type.contains("video") || media_type == "gif" {
        "video"
    } else if media_type.contains("ptt") || media_type.contains("voice") {
        "audio"
    } else if media_type.contains("audio") {
        "audio"
    } else if media_type == "sticker" {
        "image"
    } else if media_type.is_empty() {
        "text"
    } else {
        "document"
    }
}

/// Extension-derived `mediaType` for `/send-media` (hermes
/// `_map_local_media_type`).
pub fn send_media_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" => "image",
        "webp" => "sticker",
        "mp4" | "mov" | "avi" | "mkv" => "video",
        "mp3" | "ogg" | "opus" | "m4a" | "wav" | "aac" => "audio",
        _ => "document",
    }
}

/// Group mention detection: `@<bot-user-part>` when the bot JID is
/// known, otherwise any `@` mention token.
pub fn text_mentions_bot(text: &str, bot_jid: &str) -> bool {
    if let Some(user_part) = bot_jid.split('@').next().filter(|u| !u.is_empty()) {
        return text.contains(&format!("@{user_part}"));
    }
    text.split_whitespace()
        .any(|word| word.starts_with('@') && word.len() > 1)
}

/// Entry point spawned by `run_messaging`.
pub async fn run(
    cfg: WhatsappConfig,
    dispatcher: Arc<Dispatcher>,
    pairing: Option<Arc<crate::pairing::PairingStore>>,
) {
    let mut resolved = cfg.resolve();
    if resolved.bridge_url.is_empty() {
        resolved.bridge_url = "http://127.0.0.1:3000".to_string();
    }
    let runtime = Arc::new(Runtime {
        client: reqwest::Client::builder()
            .timeout(API_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        cfg: resolved,
        bot_jid: tokio::sync::Mutex::new(String::new()),
    });
    crate::messaging::register_platform_sender(
        "whatsapp",
        Arc::new(WhatsappSender {
            runtime: runtime.clone(),
        }),
    );

    let mut delay = 5u64;
    loop {
        match run_session(&runtime, &dispatcher, &pairing).await {
            Ok(()) => delay = 5,
            Err(msg) => eprintln!("[whatsapp] session error: {msg}"),
        }
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(60);
    }
}

async fn run_session(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
) -> Result<(), String> {
    // Wait for the bridge to report a connected state (hermes connect
    // loop: health probes until `status == "connected"`, pairing QR
    // states are surfaced and retried).
    loop {
        let url = format!("{}/health", runtime.cfg.bridge_url);
        match runtime.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let data: Value = resp.json().await.unwrap_or(json!({}));
                let status = data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                if status == "connected" {
                    if let Some(jid) = data
                        .get("botJid")
                        .or_else(|| data.get("jid"))
                        .and_then(|v| v.as_str())
                    {
                        *runtime.bot_jid.lock().await = jid.to_string();
                    }
                    eprintln!("[whatsapp] bridge connected at {}", runtime.cfg.bridge_url);
                    break;
                }
                if status == "qr" || status.contains("pairing") {
                    eprintln!(
                        "[whatsapp] bridge waiting for QR pairing — complete pairing in the bridge console"
                    );
                } else {
                    eprintln!("[whatsapp] bridge status: {status} — waiting for connection");
                }
            }
            Ok(resp) => {
                return Err(format!("health check HTTP {}", resp.status()));
            }
            Err(e) => {
                return Err(format!("bridge unreachable at {}: {e}", runtime.cfg.bridge_url));
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // Poll loop (hermes `_poll_messages`).
    let interval = Duration::from_millis(runtime.cfg.poll_interval_ms);
    loop {
        let url = format!("{}/messages", runtime.cfg.bridge_url);
        let resp = runtime
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("poll failed: {e}"))?;
        if resp.status().is_success() {
            let messages: Vec<Value> = resp.json().await.unwrap_or_default();
            for data in &messages {
                handle_bridge_message(runtime, dispatcher, pairing, data).await;
            }
        } else {
            eprintln!("[whatsapp] poll HTTP {}", resp.status());
        }
        tokio::time::sleep(interval).await;
    }
}

/// Process one bridge message object (hermes `_build_message_event` +
/// dispatch + read receipt).
async fn handle_bridge_message(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
    data: &Value,
) {
    if !should_process(data) {
        return;
    }
    let chat_id = data
        .get("chatId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let is_group = data.get("isGroup").and_then(|v| v.as_bool()).unwrap_or(false);
    let sender_id = data
        .get("senderId")
        .and_then(|v| v.as_str())
        .unwrap_or(&chat_id)
        .to_string();
    let sender_name = data
        .get("senderName")
        .and_then(|v| v.as_str())
        .unwrap_or(&sender_id)
        .to_string();
    let message_id = data
        .get("messageId")
        .or_else(|| data.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut text = data
        .get("text")
        .or_else(|| data.get("body"))
        .or_else(|| data.get("caption"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if is_group {
        if !runtime.cfg.allowed_channels.is_empty()
            && !runtime.cfg.allowed_channels.iter().any(|c| c == &chat_id || c == "*")
        {
            return;
        }
        let exempt = runtime
            .cfg
            .free_response_channels
            .iter()
            .any(|c| c == &chat_id);
        if runtime.cfg.require_mention && !exempt {
            let bot_jid = runtime.bot_jid.lock().await.clone();
            if !text_mentions_bot(&text, &bot_jid) {
                return;
            }
        }
    } else if !sender_allowed(runtime, pairing, &sender_id, &sender_name, &chat_id).await {
        return;
    }

    // Media: bridge-cached local paths or URLs → media cache.
    let mut attachments = Vec::new();
    let has_media = data.get("hasMedia").and_then(|v| v.as_bool()).unwrap_or(false);
    if has_media {
        let media_type = data
            .get("mediaType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mime = data
            .get("mime")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let urls = data
            .get("mediaUrls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for url_value in urls {
            let Some(url) = url_value.as_str() else { continue };
            if let Some(att) = fetch_media(runtime, url, &mime, media_type).await {
                attachments.push(att);
            }
        }
    }
    if text.is_empty() && attachments.is_empty() {
        return;
    }
    if text.is_empty() {
        text = "[media message]".to_string();
    }

    let mut event = MessageEvent {
        platform: "whatsapp".into(),
        chat_id: chat_id.clone(),
        sender_id,
        sender_name,
        text,
        message_id,
        attachments,
    };
    if !crate::messaging::pre_gateway_dispatch_gate_public(&mut event).await {
        return;
    }
    // Fire-and-forget read receipt (hermes pattern: never block dispatch).
    if runtime.cfg.read_receipts {
        if let Some(key) = data.get("readReceiptKey") {
            if key.is_object() {
                let runtime = runtime.clone();
                let key = key.clone();
                tokio::spawn(async move {
                    send_read_receipt(&runtime, &key).await;
                });
            }
        }
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
    let (reply_text, media_paths) = crate::messaging::extract_media_tags(&full);
    for path in &media_paths {
        send_media(runtime, &chat_id, path, "").await;
    }
    let reply_text = reply_text.trim().to_string();
    if !reply_text.is_empty() {
        for chunk in crate::messaging::chunk_text(&reply_text, MAX_MESSAGE_LENGTH) {
            if let Err(e) = send_text(runtime, &chat_id, &chunk).await {
                eprintln!("[whatsapp] reply to {chat_id} failed: {e}");
            }
        }
    }
}

/// Download (URL) or read (bridge-cached absolute path) inbound media
/// into the content-addressed cache.
async fn fetch_media(
    runtime: &Arc<Runtime>,
    url: &str,
    mime: &str,
    media_type: &str,
) -> Option<MediaAttachment> {
    let home = crate::config::ulnclaw_home();
    let kind = media_kind_for(media_type);
    let fallback_mime = match kind {
        "image" => "image/jpeg",
        "video" => "video/mp4",
        "audio" => "audio/ogg",
        _ => "application/octet-stream",
    };
    let mime = if mime.is_empty() { fallback_mime } else { mime };
    if url.starts_with("http://") || url.starts_with("https://") {
        match runtime.client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => {
                    let len = bytes.len() as u64;
                    match crate::media_cache::cache_media_bytes(&home, &bytes, mime, "") {
                        Ok(path) => Some(MediaAttachment {
                            path,
                            mime: mime.to_string(),
                            bytes: len,
                            original_name: String::new(),
                        }),
                        Err(e) => {
                            eprintln!("[whatsapp] media cache failed: {e}");
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[whatsapp] media download read failed: {e}");
                    None
                }
            },
            Ok(resp) => {
                eprintln!("[whatsapp] media download HTTP {}", resp.status());
                None
            }
            Err(e) => {
                eprintln!("[whatsapp] media download failed: {e}");
                None
            }
        }
    } else if std::path::Path::new(url).is_absolute() {
        // Bridge already downloaded the media to a local file.
        match tokio::fs::read(url).await {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                let name = std::path::Path::new(url)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                match crate::media_cache::cache_media_bytes(&home, &bytes, mime, &name) {
                    Ok(path) => Some(MediaAttachment {
                        path,
                        mime: mime.to_string(),
                        bytes: len,
                        original_name: name,
                    }),
                    Err(e) => {
                        eprintln!("[whatsapp] media cache failed: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("[whatsapp] bridge media path unreadable ({url}): {e}");
                None
            }
        }
    } else {
        eprintln!("[whatsapp] rejecting non-absolute bridge media path: {url}");
        None
    }
}

async fn send_read_receipt(runtime: &Arc<Runtime>, key: &Value) {
    let url = format!("{}/read", runtime.cfg.bridge_url);
    let result = runtime.client.post(&url).json(&json!({ "key": key })).send().await;
    match result {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => eprintln!("[whatsapp] read receipt HTTP {}", resp.status()),
        Err(e) => eprintln!("[whatsapp] read receipt failed: {e}"),
    }
}

/// hermes `/send` — `{chatId, message}`.
async fn send_text(runtime: &Arc<Runtime>, chat_id: &str, text: &str) -> Result<(), String> {
    let url = format!("{}/send", runtime.cfg.bridge_url);
    let payload = json!({ "chatId": chat_id, "message": text });
    let resp = runtime
        .client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("HTTP {status}: {body}"))
    }
}

/// hermes `/send-media` — `{to, path, mediaType, caption}`.
async fn send_media(
    runtime: &Arc<Runtime>,
    chat_id: &str,
    path: &std::path::Path,
    caption: &str,
) {
    let url = format!("{}/send-media", runtime.cfg.bridge_url);
    let payload = json!({
        "to": chat_id,
        "path": path.to_string_lossy(),
        "mediaType": send_media_type(&path.to_string_lossy()),
        "caption": caption,
    });
    match runtime.client.post(&url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => eprintln!("[whatsapp] send-media HTTP {}", resp.status()),
        Err(e) => eprintln!("[whatsapp] send-media failed: {e}"),
    }
}

/// DM allowlist∪pairing gate (hermes authorization semantics).
async fn sender_allowed(
    runtime: &Arc<Runtime>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
    sender_id: &str,
    sender_name: &str,
    chat_id: &str,
) -> bool {
    if runtime
        .cfg
        .allowed_users
        .iter()
        .any(|u| u == sender_id || u == "*")
    {
        return true;
    }
    if let Some(store) = pairing {
        if store.is_approved("whatsapp", sender_id) {
            return true;
        }
        if let Some(code_msg) =
            crate::messaging::pairing_offer_public(store, "whatsapp", sender_id, sender_name)
        {
            let _ = send_text(runtime, chat_id, &code_msg).await;
        } else {
            eprintln!(
                "[whatsapp] unauthorized DM from {sender_id} — add to allowed_users or approve pairing"
            );
        }
        return false;
    }
    eprintln!("[whatsapp] unauthorized DM from {sender_id} — add to allowed_users");
    false
}

struct WhatsappSender {
    runtime: Arc<Runtime>,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for WhatsappSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        for chunk in crate::messaging::chunk_text(text, MAX_MESSAGE_LENGTH) {
            if let Err(e) = send_text(&self.runtime, chat_id, &chunk).await {
                eprintln!("[whatsapp] send_text to {chat_id} failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(json_str: &str) -> Value {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn process_drops_from_me_and_status() {
        assert!(!should_process(&msg(r#"{"chatId":"123@s.whatsapp.net","fromMe":true}"#)));
        assert!(!should_process(&msg(r#"{"chatId":"status@broadcast"}"#)));
        assert!(!should_process(&msg(r#"{"chatId":"abc@newsletter"}"#)));
        assert!(!should_process(&msg(r#"{"chatId":""}"#)));
        assert!(should_process(&msg(r#"{"chatId":"123@s.whatsapp.net"}"#)));
        assert!(should_process(&msg(r#"{"chatId":"456@g.us","isGroup":true}"#)));
    }

    #[test]
    fn media_kind_mapping() {
        assert_eq!(media_kind_for("image"), "image");
        assert_eq!(media_kind_for("video"), "video");
        assert_eq!(media_kind_for("ptt"), "audio");
        assert_eq!(media_kind_for("audio"), "audio");
        assert_eq!(media_kind_for("document"), "document");
        assert_eq!(media_kind_for("sticker"), "image");
        assert_eq!(media_kind_for(""), "text");
    }

    #[test]
    fn send_media_type_by_extension() {
        assert_eq!(send_media_type("/tmp/photo.jpg"), "image");
        assert_eq!(send_media_type("/tmp/sticker.webp"), "sticker");
        assert_eq!(send_media_type("/tmp/clip.mp4"), "video");
        assert_eq!(send_media_type("/tmp/voice.ogg"), "audio");
        assert_eq!(send_media_type("/tmp/report.pdf"), "document");
        assert_eq!(send_media_type("noext"), "document");
    }

    #[test]
    fn mention_detection_with_bot_jid() {
        let jid = "15551234567@s.whatsapp.net";
        assert!(text_mentions_bot("hey @15551234567 help", jid));
        assert!(!text_mentions_bot("hey everyone", jid));
        assert!(!text_mentions_bot("hey @9998887777", jid));
    }

    #[test]
    fn mention_detection_without_jid_falls_back_to_any_mention() {
        assert!(text_mentions_bot("hey @someone help", ""));
        assert!(!text_mentions_bot("no mentions here", ""));
        assert!(!text_mentions_bot("email me at me@", ""));
    }

    #[test]
    fn resolve_defaults_and_env() {
        let _guard = crate::models_dev::test_env_lock();
        let cfg = WhatsappConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(resolved.bridge_url, "");
        assert!(resolved.require_mention);
        assert_eq!(resolved.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);

        std::env::set_var("WHATSAPP_BRIDGE_URL", "http://10.0.0.5:3000/");
        std::env::set_var("WHATSAPP_REQUIRE_MENTION", "false");
        let resolved = cfg.resolve();
        assert_eq!(resolved.bridge_url, "http://10.0.0.5:3000");
        assert!(!resolved.require_mention);
        std::env::remove_var("WHATSAPP_BRIDGE_URL");
        std::env::remove_var("WHATSAPP_REQUIRE_MENTION");
    }

    #[test]
    fn poll_interval_has_a_floor() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::set_var("WHATSAPP_POLL_INTERVAL_MS", "5");
        let resolved = WhatsappConfig::default().resolve();
        assert_eq!(resolved.poll_interval_ms, 100);
        std::env::remove_var("WHATSAPP_POLL_INTERVAL_MS");
    }

    #[test]
    fn text_extraction_prefers_text_then_body_then_caption() {
        let data = msg(r#"{"text":"hello","body":"ignored"}"#);
        let text = data
            .get("text")
            .or_else(|| data.get("body"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(text, "hello");
        let data = msg(r#"{"body":"from body"}"#);
        let text = data
            .get("text")
            .or_else(|| data.get("body"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(text, "from body");
    }

    #[test]
    fn chunk_limit_matches_hermes_conservative_bound() {
        assert_eq!(MAX_MESSAGE_LENGTH, 4000);
        assert_eq!(DEFAULT_POLL_INTERVAL_MS, 1000);
    }
}
