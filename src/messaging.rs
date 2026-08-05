//! Messaging platform gateways — port of hermes `gateway/platforms/`.
//!
//! Hermes runs chat platforms (Telegram/Discord/Slack/…) inside the
//! gateway process: each adapter normalizes incoming messages into a
//! `MessageEvent`, the core runs an agent turn per chat, and the reply
//! goes back through the platform. ulnclaw ports the architecture plus
//! three self-contained adapters:
//!
//! * **Telegram** — Bot API long-polling (`getUpdates` / `sendMessage`)
//! * **Discord**  — Gateway v10 websocket (IDENTIFY/heartbeat/
//!   MESSAGE_CREATE) + REST message send
//! * **Slack**    — Socket Mode websocket (events_api envelopes) +
//!   `chat.postMessage`
//!
//! Access control mirrors hermes pairing semantics: every platform is
//! allowlist-gated (`allowed_chat_ids` / `allowed_channel_ids`); an empty
//! allowlist refuses all traffic and logs the ids to add (fail closed).

use crate::agent::Agent;
use crate::error::{AgentError, Result};
use crate::session::sqlite::SqliteSessionStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// `[messaging]` config block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessagingConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    /// Offer pairing codes to unknown senders (hermes unauthorized-DM
    /// `pair` behavior). Approved codes join the allowlist as a union.
    #[serde(default = "default_pairing")]
    pub pairing: bool,
    /// WhatsApp Cloud webhook platform (mounted on the gateway router).
    #[serde(default)]
    pub whatsapp_cloud: crate::webhook_platforms::WhatsAppCloudConfig,
    /// Microsoft Graph change-notification ingress (gateway router).
    #[serde(default)]
    pub msgraph: crate::webhook_platforms::MsGraphConfig,
    /// Generic inbound-webhook platform: signed routes from external
    /// services (hermes `platforms.webhook`).
    #[serde(default)]
    pub webhook: crate::webhook_platforms::WebhookConfig,
}

fn default_pairing() -> bool {
    true
}

/// `[messaging.telegram]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token from @BotFather (falls back to `TELEGRAM_BOT_TOKEN`).
    #[serde(default)]
    pub bot_token: String,
    /// Chat ids allowed to talk to the bot (hermes pairing). Empty =
    /// refuse all and log the ids that try.
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
}

/// `[messaging.discord]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token (falls back to `DISCORD_BOT_TOKEN`).
    #[serde(default)]
    pub bot_token: String,
    /// Channel ids allowed to talk to the bot (empty = refuse all).
    #[serde(default)]
    pub allowed_channel_ids: Vec<String>,
}

/// `[messaging.slack]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlackConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token `xoxb-…` (falls back to `SLACK_BOT_TOKEN`).
    #[serde(default)]
    pub bot_token: String,
    /// App-level token `xapp-…` for Socket Mode (falls back to
    /// `SLACK_APP_TOKEN`).
    #[serde(default)]
    pub app_token: String,
    /// Channel ids allowed to talk to the bot (empty = refuse all).
    #[serde(default)]
    pub allowed_channel_ids: Vec<String>,
}

/// A downloaded attachment cached in `<home>/media-cache/` (hermes media
/// pipeline input).
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    pub path: std::path::PathBuf,
    pub mime: String,
    pub bytes: u64,
    pub original_name: String,
}

/// Normalized incoming message (hermes `MessageEvent`, core fields).
#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub platform: String,
    pub chat_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
    pub message_id: String,
    /// Cached media attachments (empty when the message had none or the
    /// download failed — failures degrade to a text note, never fatal).
    pub attachments: Vec<MediaAttachment>,
}

/// Per-chat conversation runner: one session per platform+chat
/// (hermes gateway session routing).
pub struct Dispatcher {
    agent: Arc<Agent>,
    store: Arc<SqliteSessionStore>,
    /// Per-chat in-flight guard: one turn at a time per chat.
    busy: Arc<Mutex<HashMap<String, bool>>>,
    /// Per-chat rolling history (mirrors the REPL continuity model).
    histories: Arc<Mutex<HashMap<String, Vec<crate::provider::Message>>>>,
}

/// Result of one dispatched platform message (hermes run-turn output):
/// the agent reply plus any 🎙️ transcript echoes to deliver first.
#[derive(Debug, Clone, Default)]
pub struct DispatchOutcome {
    pub reply: String,
    pub transcript_echoes: Vec<String>,
}

impl Dispatcher {
    pub fn new(agent: Arc<Agent>, store: Arc<SqliteSessionStore>) -> Arc<Self> {
        Arc::new(Self {
            agent,
            store,
            busy: Arc::new(Mutex::new(HashMap::new())),
            histories: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn session_key(event: &MessageEvent) -> String {
        format!("platform-{}-{}", event.platform, event.chat_id)
    }

    /// Run one agent turn for the event's chat; returns the reply text
    /// plus transcript echoes (hermes `_echo_pending_stt_transcripts_once`).
    pub async fn handle_event(self: &Arc<Self>, event: MessageEvent) -> Result<DispatchOutcome> {
        let key = Self::session_key(&event);
        {
            let mut busy = self.busy.lock().await;
            if *busy.get(&key).unwrap_or(&false) {
                return Ok(DispatchOutcome {
                    reply: "(the previous message is still being processed — please wait)".into(),
                    transcript_echoes: Vec::new(),
                });
            }
            busy.insert(key.clone(), true);
        }
        let result = self.run_turn(&key, &event).await;
        self.busy.lock().await.insert(key, false);
        result
    }

    async fn run_turn(&self, key: &str, event: &MessageEvent) -> Result<DispatchOutcome> {
        use crate::provider::Role;
        // Ensure the session row exists under a deterministic id.
        if self.store.resolve_session_id(key).ok().flatten().is_none() {
            self.store
                .create_named_session(
                    key,
                    &format!("platform:{}", event.platform),
                    Some(&self.agent.context().config.model.model),
                    None,
                )
                .ok();
        }
        let mut histories = self.histories.lock().await;
        let history = histories.entry(key.to_string()).or_default();
        if history.is_empty() {
            if let Ok(messages) = self.store.load_messages(key) {
                *history = messages
                    .into_iter()
                    .filter(|m| m.role != Role::System)
                    .collect();
            }
        }
        // Voice-note STT enrichment (hermes
        // `_transcribe_pending_audio_event_once`): audio attachments are
        // transcribed ahead of the turn; transcripts are echoed back to
        // the chat and the enriched text replaces the raw caption.
        let config = &self.agent.context().config;
        let audio_paths: Vec<std::path::PathBuf> = event
            .attachments
            .iter()
            .filter(|a| crate::stt::attachment_is_stt_input(&a.mime))
            .map(|a| a.path.clone())
            .collect();
        let mut user_text = event.text.clone();
        let mut transcript_echoes: Vec<String> = Vec::new();
        let mut transcribed: Vec<std::path::PathBuf> = Vec::new();
        if !audio_paths.is_empty() {
            let (enriched, transcripts, done) = crate::stt::enrich_message_with_transcription(
                &config.stt,
                &user_text,
                &audio_paths,
            )
            .await;
            user_text = enriched;
            transcribed = done;
            if config.stt.echo_transcripts {
                transcript_echoes = transcripts
                    .iter()
                    .map(|t| crate::stt::echo_line(t))
                    .collect();
            }
        }
        let mut prompt = if event.sender_name.is_empty() {
            user_text
        } else {
            format!("{}: {}", event.sender_name, user_text)
        };
        // Cached attachments are referenced by path so the agent can apply
        // vision_analyze / video_analyze / read_file (hermes text-fallback
        // semantics for media). Transcribed voice notes already appear in
        // the enriched text — skip them to avoid duplicate path noise.
        let remaining: Vec<&MediaAttachment> = event
            .attachments
            .iter()
            .filter(|a| !transcribed.contains(&a.path))
            .collect();
        prompt.push_str(&attachment_note_refs(&remaining));
        let result = self
            .agent
            .run_with_session(&prompt, Some(history.clone()), Some(key))
            .await?;
        *history = result
            .conversation
            .into_iter()
            .filter(|m| m.role != Role::System)
            .collect();
        // Fire the gateway pre-dispatch observers' counterpart: the
        // session hooks already cover lifecycle; keep this minimal.
        Ok(DispatchOutcome {
            reply: result.content,
            transcript_echoes,
        })
    }
}

/// Access gate (hermes pairing): allowlisted ids pass; everything else
/// is refused and reported so the user knows what to allowlist.
fn allowlisted(allowlist: &[String], id: &str) -> bool {
    allowlist.iter().any(|allowed| allowed == id)
}

/// Offer a pairing code to an unauthorized sender (hermes pairing flow).
/// Returns the reply text, or None to stay silent (rate-limited repeat).
fn pairing_offer(
    store: &crate::pairing::PairingStore,
    platform: &str,
    sender_id: &str,
    sender_name: &str,
) -> Option<String> {
    // Repeat requests within the rate window are silently ignored (hermes).
    if store.is_rate_limited(platform, sender_id) {
        return None;
    }
    match store.generate_code(platform, sender_id, sender_name) {
        Some(code) => Some(format!(
            "Hi~ I don't recognize you yet!\n\n\
             Here's your pairing code: `{code}`\n\n\
             Ask the bot owner to run:\n\
             `ulnclaw pairing approve {platform} {code}`"
        )),
        None => {
            // Queue full or locked out: send the throttle notice once, then
            // silence follow-ups via the rate limit (hermes behavior).
            store.record_rate_limit(platform, sender_id);
            Some("Too many pairing requests right now~ Please try again later!".to_string())
        }
    }
}

/// Attachment download cap (Discord's free-tier upload limit — hermes
/// applies a similar ceiling).
const MAX_MEDIA_BYTES: u64 = 25 * 1024 * 1024;

/// Cache downloaded bytes and wrap failures into a text note (hermes
/// degrades attachment problems to text, never fatal).
fn cache_attachment(
    data: Vec<u8>,
    mime: &str,
    original_name: &str,
) -> Option<MediaAttachment> {
    if data.is_empty() || data.len() as u64 > MAX_MEDIA_BYTES {
        return None;
    }
    let home = crate::config::ulnclaw_home();
    match crate::media_cache::cache_media_bytes(&home, &data, mime, original_name) {
        Ok(path) => Some(MediaAttachment {
            path,
            mime: if mime.is_empty() {
                "application/octet-stream".to_string()
            } else {
                crate::media_cache::normalize_mime(mime)
            },
            bytes: data.len() as u64,
            original_name: original_name.to_string(),
        }),
        Err(e) => {
            eprintln!("[messaging] media cache write failed: {e}");
            None
        }
    }
}

/// Download one attachment URL into the media cache. `auth` is an
/// optional Authorization header (Slack private file URLs need the bot
/// bearer). Failures degrade to None with a log line (hermes policy).
async fn download_to_cache(
    client: &reqwest::Client,
    url: &str,
    auth: Option<&str>,
    mime: &str,
    original_name: &str,
    declared_size: u64,
) -> Option<MediaAttachment> {
    if declared_size > MAX_MEDIA_BYTES {
        eprintln!(
            "[messaging] skipping attachment {original_name:?}: {declared_size} bytes exceeds the {MAX_MEDIA_BYTES} byte cap"
        );
        return None;
    }
    let mut request = client.get(url);
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    match request.send().await.and_then(|r| r.error_for_status()) {
        Ok(response) => match response.bytes().await {
            Ok(bytes) => cache_attachment(bytes.to_vec(), mime, original_name),
            Err(e) => {
                eprintln!("[messaging] attachment download failed: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("[messaging] attachment download failed: {e}");
            None
        }
    }
}

/// Render attachments as a text note appended to the user message —
/// the agent can inspect images via `vision_analyze`, video via
/// `video_analyze`, and documents via `read_file` (hermes falls back to
/// exactly this text reference when native multimodal injection fails).
#[cfg(test)]
fn attachment_note(attachments: &[MediaAttachment]) -> String {
    let refs: Vec<&MediaAttachment> = attachments.iter().collect();
    attachment_note_refs(&refs)
}

fn attachment_note_refs(attachments: &[&MediaAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut note = String::from("\n\n[Attached media]\n");
    for attachment in attachments {
        let name = if attachment.original_name.is_empty() {
            String::new()
        } else {
            format!(" \"{}\"", attachment.original_name)
        };
        note.push_str(&format!(
            "- {} ({}, {} bytes){}\n",
            attachment.path.display(),
            attachment.mime,
            attachment.bytes,
            name
        ));
    }
    note.push_str(
        "Inspect images with vision_analyze, video with video_analyze,          documents with read_file.",
    );
    note
}

/// Extract `MEDIA:<path>` delivery tags from a reply (hermes
/// `extract_media`): returns the cleaned text plus the file paths, in
/// order of appearance. Tags may stand alone on a line; surrounding
/// whitespace-only lines are dropped with them.
pub fn extract_media_tags(text: &str) -> (String, Vec<std::path::PathBuf>) {
    let mut cleaned: Vec<String> = Vec::new();
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("MEDIA:") {
            let candidate = rest.trim().trim_matches('`');
            if !candidate.is_empty() {
                let expanded = if let Some(stripped) = candidate.strip_prefix("~/") {
                    std::env::var_os("HOME")
                        .map(|home| {
                            std::path::PathBuf::from(home)
                                .join(stripped)
                                .display()
                                .to_string()
                        })
                        .unwrap_or_else(|| candidate.to_string())
                } else {
                    candidate.to_string()
                };
                if std::path::Path::new(&expanded).is_file() {
                    paths.push(std::path::PathBuf::from(expanded));
                    continue;
                }
            }
        }
        cleaned.push(line.to_string());
    }
    // Collapse runs of blank lines left behind by removed tags.
    let mut out = String::new();
    let mut prev_blank = false;
    for line in &cleaned {
        let blank = line.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = blank;
    }
    (out.trim_end_matches('\n').to_string(), paths)
}

/// Pub wrapper for webhook platforms (whatsapp_cloud / msgraph).
pub fn pairing_offer_public(
    store: &crate::pairing::PairingStore,
    platform: &str,
    sender_id: &str,
    sender_name: &str,
) -> Option<String> {
    pairing_offer(store, platform, sender_id, sender_name)
}

/// Pub wrapper for webhook platforms (whatsapp_cloud / msgraph).
pub async fn pre_gateway_dispatch_gate_public(event: &mut MessageEvent) -> bool {
    pre_gateway_dispatch_gate(event).await
}

/// `pre_gateway_dispatch` plugin hook (hermes fires it BEFORE auth so
/// plugins can handle unauthorized senders without triggering pairing).
/// Returns false when a hook consumed the event (`{"action": "skip"}`);
/// a `{"action": "rewrite", "text": ...}` response replaces `event.text`.
async fn pre_gateway_dispatch_gate(event: &mut MessageEvent) -> bool {
    if !crate::plugins::has_hook("pre_gateway_dispatch") {
        return true;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let payload = crate::plugins::hook_payload(
        "pre_gateway_dispatch",
        "",
        &cwd,
        vec![
            ("platform", json!(event.platform)),
            ("chat_id", json!(event.chat_id)),
            ("sender_id", json!(event.sender_id)),
            ("sender_name", json!(event.sender_name)),
            ("text", json!(event.text)),
            ("message_id", json!(event.message_id)),
        ],
        json!({}),
    );
    let responses = crate::plugins::invoke_hook("pre_gateway_dispatch", payload).await;
    match crate::plugins::dispatch_decision(&responses) {
        Some((action, detail)) if action == "skip" => {
            eprintln!(
                "[messaging] pre_gateway_dispatch skipped {} message from chat {}: {}",
                event.platform,
                event.chat_id,
                detail.unwrap_or_default()
            );
            false
        }
        Some((action, text)) if action == "rewrite" => {
            if let Some(text) = text {
                event.text = text;
            }
            true
        }
        _ => true,
    }
}

/// Start every enabled platform; resolves when they have all stopped.
pub async fn run_messaging(
    config: &crate::config::UlncLawConfig,
    agent: Arc<Agent>,
    store: Arc<SqliteSessionStore>,
) {
    let dispatcher = Dispatcher::new(agent, store);
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let msg = &config.messaging;
    let pairing: Option<Arc<crate::pairing::PairingStore>> = if msg.pairing {
        Some(Arc::new(crate::pairing::PairingStore::open(
            &crate::config::ulnclaw_home(),
        )))
    } else {
        None
    };
    if msg.telegram.enabled {
        let cfg = msg.telegram.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            telegram::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.discord.enabled {
        let cfg = msg.discord.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            discord::run(cfg, dispatcher, pairing).await;
        }));
    }
    if msg.slack.enabled {
        let cfg = msg.slack.clone();
        let dispatcher = dispatcher.clone();
        let pairing = pairing.clone();
        tasks.push(tokio::spawn(async move {
            slack::run(cfg, dispatcher, pairing).await;
        }));
    }
    if tasks.is_empty() {
        eprintln!("[messaging] no platforms enabled ([messaging.telegram|discord|slack] enabled = true)");
        return;
    }
    for task in tasks {
        task.await.ok();
    }
}

fn resolve_token(configured: &str, env_var: &str) -> Option<String> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        return Some(trimmed.to_string());
    }
    std::env::var(env_var).ok().filter(|v| !v.trim().is_empty())
}

/// Public token resolution for cross-module delivery targets
/// (generic-webhook `deliver = "telegram"`; hermes webhook.py reuses the
/// platform credentials).
pub fn resolve_telegram_token_public(cfg: &TelegramConfig) -> Option<String> {
    resolve_token(&cfg.bot_token, "TELEGRAM_BOT_TOKEN")
}

/// Public send wrapper (see `resolve_telegram_token_public`).
pub async fn telegram_send_public(client: &reqwest::Client, token: &str, chat_id: &str, text: &str) {
    telegram::send_message(client, token, chat_id, text).await
}

/// Public token resolution for webhook `deliver = "discord"`.
pub fn resolve_discord_token_public(cfg: &DiscordConfig) -> Option<String> {
    resolve_token(&cfg.bot_token, "DISCORD_BOT_TOKEN")
}

/// Public send wrapper (see `resolve_discord_token_public`).
pub async fn discord_send_public(token: &str, channel_id: &str, text: &str) {
    discord::send_channel_message(token, channel_id, text).await
}

/// Public token resolution for webhook `deliver = "slack"`.
pub fn resolve_slack_bot_token_public(cfg: &SlackConfig) -> Option<String> {
    resolve_token(&cfg.bot_token, "SLACK_BOT_TOKEN")
}

/// Public send wrapper (see `resolve_slack_bot_token_public`).
pub async fn slack_send_public(token: &str, channel: &str, text: &str) {
    slack::post_message(token, channel, text).await
}

// ---------------------------------------------------------------------------
// Telegram — Bot API long-polling
// ---------------------------------------------------------------------------

pub mod telegram {
    use super::*;

    const API: &str = "https://api.telegram.org";

    pub async fn run(cfg: TelegramConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) {
        let Some(token) = resolve_token(&cfg.bot_token, "TELEGRAM_BOT_TOKEN") else {
            eprintln!("[telegram] disabled: no bot_token configured (set messaging.telegram.bot_token or TELEGRAM_BOT_TOKEN)");
            return;
        };
        let client = reqwest::Client::new();
        match api(&client, &token, "getMe", json!({})).await {
            Ok(me) => {
                let username = me.pointer("/result/username").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("[telegram] connected as @{username}");
            }
            Err(e) => {
                eprintln!("[telegram] getMe failed: {e}");
                return;
            }
        }
        let mut offset: Option<i64> = None;
        loop {
            let mut params = json!({"timeout": 25, "allowed_updates": ["message"]});
            if let Some(offset) = offset {
                params["offset"] = json!(offset + 1);
            }
            let updates = match api(&client, &token, "getUpdates", params).await {
                Ok(value) => value
                    .pointer("/result")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                Err(e) => {
                    eprintln!("[telegram] getUpdates failed: {e} — retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };
            for update in updates {
                let update_id = update.get("update_id").and_then(|v| v.as_i64()).unwrap_or(0);
                offset = Some(offset.unwrap_or(0).max(update_id));
                let Some(message) = update.get("message") else { continue };
                let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("");
                // Media (photo/document/video/audio/voice) is downloaded
                // and cached; media-only messages still flow (hermes).
                let attachments = download_media(&client, &token, message).await;
                if text.is_empty() && attachments.is_empty() {
                    continue;
                }
                let chat = message.get("chat").cloned().unwrap_or(json!({}));
                let chat_id = chat
                    .get("id")
                    .map(|v| match v {
                        Value::Number(n) => n.to_string(),
                        other => other.as_str().unwrap_or("").to_string(),
                    })
                    .unwrap_or_default();
                if chat_id.is_empty() {
                    continue;
                }
                let sender = message.get("from").cloned().unwrap_or(json!({}));
                let mut event = MessageEvent {
                    platform: "telegram".into(),
                    chat_id: chat_id.clone(),
                    sender_id: sender.get("id").map(|v| v.to_string()).unwrap_or_default(),
                    sender_name: sender
                        .get("first_name")
                        .or_else(|| sender.get("username"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    text: text.to_string(),
                    message_id: message
                        .get("message_id")
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    attachments,
                };
                // Plugin gate before auth (hermes ordering).
                if !pre_gateway_dispatch_gate(&mut event).await {
                    continue;
                }
                // Auth union: configured allowlist OR an approved pairing
                // code (hermes authz union).
                let authorized = allowlisted(&cfg.allowed_chat_ids, &chat_id)
                    || pairing
                        .as_ref()
                        .map(|store| store.is_approved("telegram", &event.sender_id))
                        .unwrap_or(false);
                if !authorized {
                    eprintln!(
                        "[telegram] refusing message from chat {chat_id} — add it to \
                         messaging.telegram.allowed_chat_ids or approve a pairing code"
                    );
                    if let Some(store) = pairing.as_ref() {
                        if let Some(reply) = pairing_offer(store, "telegram", &event.sender_id, &event.sender_name) {
                            send_message(&client, &token, &chat_id, &reply).await;
                        }
                    }
                    continue;
                }
                let dispatcher = dispatcher.clone();
                let client = client.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    let outcome = match dispatcher.handle_event(event.clone()).await {
                        Ok(outcome) => outcome,
                        Err(e) => crate::messaging::DispatchOutcome {
                            reply: format!("error: {e}"),
                            transcript_echoes: Vec::new(),
                        },
                    };
                    for echo in &outcome.transcript_echoes {
                        send_message(&client, &token, &event.chat_id, echo).await;
                    }
                    let reply = outcome.reply;
                    // MEDIA:<path> tags become native attachments (hermes
                    // extract_media); the rest is sent as text.
                    let (reply_text, media_paths) = extract_media_tags(&reply);
                    if !reply_text.trim().is_empty() {
                        send_message(&client, &token, &event.chat_id, &reply_text).await;
                    }
                    for path in &media_paths {
                        if crate::media_cache::mime_for_ext(path).starts_with("image/") {
                            send_photo(&client, &token, &event.chat_id, path).await;
                        } else {
                            send_document(&client, &token, &event.chat_id, path).await;
                        }
                    }
                });
            }
        }
    }

    async fn api(client: &reqwest::Client, token: &str, method: &str, params: Value) -> Result<Value> {
        let url = format!("{API}/bot{token}/{method}");
        let response = client
            .post(&url)
            .json(&params)
            .timeout(std::time::Duration::from_secs(35))
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("telegram {method}: {e}")))?;
        let value: Value = response
            .json()
            .await
            .map_err(|e| AgentError::Tool(format!("telegram {method} parse: {e}")))?;
        if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(value)
        } else {
            Err(AgentError::Tool(format!(
                "telegram {method}: {}",
                value.get("description").and_then(|v| v.as_str()).unwrap_or("unknown error")
            )))
        }
    }

    /// Send a reply, splitting at 4096-char Telegram limit (hermes chunks
    /// long replies the same way).
    /// Download the message's first media payload (photo > document >
    /// video > audio > voice, hermes precedence) via getFile and cache it.
    async fn download_media(
        client: &reqwest::Client,
        token: &str,
        message: &Value,
    ) -> Vec<MediaAttachment> {
        let (file_id, mime, name): (String, &str, String) =
            if let Some(photo) = message.get("photo").and_then(|v| v.as_array()) {
                // Bot API sends photos as a size ladder; last = largest.
                match photo.last().and_then(|p| p.get("file_id").and_then(|v| v.as_str())) {
                    Some(file_id) => (file_id.to_string(), "image/jpeg", String::new()),
                    None => return Vec::new(),
                }
            } else if let Some(media) = message
                .get("document")
                .or_else(|| message.get("video"))
                .or_else(|| message.get("audio"))
                .or_else(|| message.get("voice"))
            {
                let Some(file_id) = media.get("file_id").and_then(|v| v.as_str()) else {
                    return Vec::new();
                };
                let mime = if message.get("voice").is_some() {
                    "audio/ogg"
                } else {
                    media.get("mime_type").and_then(|v| v.as_str()).unwrap_or("")
                };
                let name = media
                    .get("file_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (file_id.to_string(), mime, name)
            } else {
                return Vec::new();
            };
        let info = match api(client, token, "getFile", json!({"file_id": file_id})).await {
            Ok(info) => info,
            Err(e) => {
                eprintln!("[telegram] getFile failed: {e}");
                return Vec::new();
            }
        };
        let Some(file_path) = info.pointer("/result/file_path").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let url = format!("{API}/file/bot{token}/{file_path}");
        match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
            Ok(response) => match response.bytes().await {
                Ok(bytes) => cache_attachment(bytes.to_vec(), mime, &name)
                    .into_iter()
                    .collect(),
                Err(e) => {
                    eprintln!("[telegram] media download failed: {e}");
                    Vec::new()
                }
            },
            Err(e) => {
                eprintln!("[telegram] media download failed: {e}");
                Vec::new()
            }
        }
    }

    /// Send a photo (MEDIA: delivery, hermes extract_media path).
    pub async fn send_photo(client: &reqwest::Client, token: &str, chat_id: &str, path: &std::path::Path) {
        let Ok(data) = tokio::fs::read(path).await else {
            eprintln!("[telegram] cannot read media {}", path.display());
            return;
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "media".to_string());
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", reqwest::multipart::Part::bytes(data).file_name(file_name));
        let url = format!("{API}/bot{token}/sendPhoto");
        if let Err(e) = client.post(&url).multipart(form).send().await {
            eprintln!("[telegram] sendPhoto failed: {e}");
        }
    }

    /// Send a non-image file (MEDIA: delivery).
    pub async fn send_document(client: &reqwest::Client, token: &str, chat_id: &str, path: &std::path::Path) {
        let Ok(data) = tokio::fs::read(path).await else {
            eprintln!("[telegram] cannot read media {}", path.display());
            return;
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "media".to_string());
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", reqwest::multipart::Part::bytes(data).file_name(file_name));
        let url = format!("{API}/bot{token}/sendDocument");
        if let Err(e) = client.post(&url).multipart(form).send().await {
            eprintln!("[telegram] sendDocument failed: {e}");
        }
    }

    pub async fn send_message(client: &reqwest::Client, token: &str, chat_id: &str, text: &str) {
        for chunk in chunk_text(text, 4000) {
            let params = json!({"chat_id": chat_id, "text": chunk});
            if let Err(e) = api(client, token, "sendMessage", params).await {
                eprintln!("[telegram] sendMessage failed: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Discord — Gateway v10 websocket + REST
// ---------------------------------------------------------------------------

pub mod discord {
    use super::*;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
    const REST: &str = "https://discord.com/api/v10";
    const INTENT_GUILD_MESSAGES: u64 = 1 << 9;
    const INTENT_DIRECT_MESSAGES: u64 = 1 << 12;
    const INTENT_MESSAGE_CONTENT: u64 = 1 << 15;

    pub async fn run(cfg: DiscordConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) {
        let Some(token) = resolve_token(&cfg.bot_token, "DISCORD_BOT_TOKEN") else {
            eprintln!("[discord] disabled: no bot_token configured (set messaging.discord.bot_token or DISCORD_BOT_TOKEN)");
            return;
        };
        loop {
            if let Err(e) = run_session(&cfg, &token, dispatcher.clone(), pairing.clone()).await {
                eprintln!("[discord] gateway session ended: {e} — reconnecting in 5s");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn run_session(cfg: &DiscordConfig, token: &str, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) -> Result<()> {
        use futures::{SinkExt, StreamExt};
        let (ws, _) = tokio_tungstenite::connect_async(GATEWAY)
            .await
            .map_err(|e| AgentError::Tool(format!("discord connect: {e}")))?;
        eprintln!("[discord] gateway connected");
        let (mut sink, mut stream) = ws.split();
        let mut heartbeat_interval = std::time::Duration::from_secs(41);
        let mut last_sequence: Option<u64> = None;
        let mut identified = false;
        let mut heartbeat_due = tokio::time::Instant::now() + heartbeat_interval;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(heartbeat_due) => {
                    let payload = json!({"op": 1, "d": last_sequence});
                    sink.send(WsMessage::Text(payload.to_string())).await.ok();
                    heartbeat_due = tokio::time::Instant::now() + heartbeat_interval;
                }
                message = stream.next() => {
                    let Some(Ok(message)) = message else {
                        return Err(AgentError::Tool("discord gateway closed".into()));
                    };
                    let WsMessage::Text(text) = message else { continue };
                    let Ok(payload) = serde_json::from_str::<Value>(&text) else { continue };
                    let op = payload.get("op").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
                    if let Some(seq) = payload.get("s").and_then(|v| v.as_u64()) {
                        last_sequence = Some(seq);
                    }
                    match op {
                        10 => {
                            heartbeat_interval = std::time::Duration::from_millis(
                                payload.pointer("/d/heartbeat_interval").and_then(|v| v.as_u64()).unwrap_or(41250),
                            );
                            heartbeat_due = tokio::time::Instant::now() + heartbeat_interval;
                            if !identified {
                                let identify = json!({
                                    "op": 2,
                                    "d": {
                                        "token": token,
                                        "intents": INTENT_GUILD_MESSAGES | INTENT_DIRECT_MESSAGES | INTENT_MESSAGE_CONTENT,
                                        "properties": {"os": "linux", "browser": "ulnclaw", "device": "ulnclaw"},
                                    }
                                });
                                sink.send(WsMessage::Text(identify.to_string())).await.ok();
                                identified = true;
                            }
                        }
                        11 => {} // heartbeat ACK
                        1 => {
                            let payload = json!({"op": 1, "d": last_sequence});
                            sink.send(WsMessage::Text(payload.to_string())).await.ok();
                        }
                        0 => {
                            let event_name = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
                            if event_name == "READY" {
                                let username = payload.pointer("/d/user/username").and_then(|v| v.as_str()).unwrap_or("?");
                                eprintln!("[discord] logged in as {username}");
                            }
                            if event_name == "MESSAGE_CREATE" {
                                handle_message_create(cfg, token, &dispatcher, payload.get("d").cloned().unwrap_or(json!({})), pairing.as_deref()).await;
                            }
                        }
                        7 | 9 => {
                            return Err(AgentError::Tool("discord requested reconnect / invalid session".into()));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn handle_message_create(cfg: &DiscordConfig, token: &str, dispatcher: &Arc<Dispatcher>, data: Value, pairing: Option<&crate::pairing::PairingStore>) {
        // Ignore bot/webhook messages (never talk to ourselves).
        if data.get("author").and_then(|a| a.get("bot")).and_then(|v| v.as_bool()).unwrap_or(false) {
            return;
        }
        let text = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        // Attachment downloads run before any gate so media-only messages
        // still flow (hermes). Discord exposes url/filename/content_type.
        let attachments = {
            let client = reqwest::Client::new();
            let mut downloaded = Vec::new();
            if let Some(items) = data.get("attachments").and_then(|v| v.as_array()) {
                for item in items {
                    let Some(url) = item.get("url").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let name = item.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                    let mime = item.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                    let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some(attachment) =
                        download_to_cache(&client, url, None, mime, name, size).await
                    {
                        downloaded.push(attachment);
                    }
                }
            }
            downloaded
        };
        if text.is_empty() && attachments.is_empty() {
            return;
        }
        let channel_id = data.get("channel_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if channel_id.is_empty() {
            return;
        }
        let author = data.get("author").cloned().unwrap_or(json!({}));
        let mut event = MessageEvent {
            platform: "discord".into(),
            chat_id: channel_id.clone(),
            sender_id: author.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            sender_name: author
                .get("global_name")
                .or_else(|| author.get("username"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            text: text.to_string(),
            message_id: data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            attachments,
        };
        // Plugin gate before auth (hermes ordering).
        if !pre_gateway_dispatch_gate(&mut event).await {
            return;
        }
        // Auth union: configured allowlist OR an approved pairing code.
        let authorized = allowlisted(&cfg.allowed_channel_ids, &channel_id)
            || pairing
                .map(|store| store.is_approved("discord", &event.sender_id))
                .unwrap_or(false);
        if !authorized {
            eprintln!(
                "[discord] refusing message from channel {channel_id} — add it to \
                 messaging.discord.allowed_channel_ids or approve a pairing code"
            );
            if let Some(store) = pairing {
                if let Some(reply) = pairing_offer(store, "discord", &event.sender_id, &event.sender_name) {
                    send_channel_message(token, &channel_id, &reply).await;
                }
            }
            return;
        }
        let dispatcher = dispatcher.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            let outcome = match dispatcher.handle_event(event.clone()).await {
                Ok(outcome) => outcome,
                Err(e) => crate::messaging::DispatchOutcome {
                    reply: format!("error: {e}"),
                    transcript_echoes: Vec::new(),
                },
            };
            for echo in &outcome.transcript_echoes {
                send_channel_message(&token, &event.chat_id, echo).await;
            }
            let reply = outcome.reply;
            let (reply_text, media_paths) = extract_media_tags(&reply);
            if !reply_text.trim().is_empty() {
                send_channel_message(&token, &event.chat_id, &reply_text).await;
            }
            for path in &media_paths {
                send_attachment(&token, &event.chat_id, path).await;
            }
        });
    }

    /// Upload a file as a native Discord attachment (MEDIA: delivery).
    async fn send_attachment(token: &str, channel_id: &str, path: &std::path::Path) {
        let Ok(data) = tokio::fs::read(path).await else {
            eprintln!("[discord] cannot read media {}", path.display());
            return;
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "media".to_string());
        let form = reqwest::multipart::Form::new()
            .part("files[0]", reqwest::multipart::Part::bytes(data).file_name(file_name));
        let url = format!("https://discord.com/api/v10/channels/{channel_id}/messages");
        let response = reqwest::Client::new()
            .post(&url)
            .header("Authorization", format!("Bot {token}"))
            .multipart(form)
            .send()
            .await;
        if let Err(e) = response.and_then(|r| r.error_for_status()) {
            eprintln!("[discord] attachment upload failed: {e}");
        }
    }

    /// POST /channels/{id}/messages with 2000-char Discord chunking.
    pub async fn send_channel_message(token: &str, channel_id: &str, text: &str) {
        let client = reqwest::Client::new();
        for chunk in chunk_text(text, 1900) {
            let result = client
                .post(format!("{REST}/channels/{channel_id}/messages"))
                .header("Authorization", format!("Bot {token}"))
                .json(&json!({"content": chunk}))
                .send()
                .await;
            if let Err(e) = result {
                eprintln!("[discord] send failed: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Slack — Socket Mode websocket + chat.postMessage
// ---------------------------------------------------------------------------

pub mod slack {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    pub async fn run(cfg: SlackConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<crate::pairing::PairingStore>>) {
        let Some(bot_token) = resolve_token(&cfg.bot_token, "SLACK_BOT_TOKEN") else {
            eprintln!("[slack] disabled: no bot_token configured (set messaging.slack.bot_token or SLACK_BOT_TOKEN)");
            return;
        };
        let Some(app_token) = resolve_token(&cfg.app_token, "SLACK_APP_TOKEN") else {
            eprintln!("[slack] disabled: no app_token configured (set messaging.slack.app_token or SLACK_APP_TOKEN)");
            return;
        };
        loop {
            match run_socket_session(&cfg, &bot_token, &app_token, dispatcher.clone(), pairing.clone()).await {
                Ok(()) => {}
                Err(e) => eprintln!("[slack] socket session ended: {e} — reconnecting in 5s"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn open_socket_url(client: &reqwest::Client, app_token: &str) -> Result<String> {
        let response = client
            .post("https://slack.com/api/apps.connections.open")
            .header("Authorization", format!("Bearer {app_token}"))
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("slack open: {e}")))?;
        let value: Value = response.json().await.map_err(|e| AgentError::Tool(format!("slack open parse: {e}")))?;
        if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(AgentError::Tool(format!(
                "slack apps.connections.open: {}",
                value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
            )));
        }
        value
            .get("url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| AgentError::Tool("slack open: no url".into()))
    }

    async fn run_socket_session(
        cfg: &SlackConfig,
        bot_token: &str,
        app_token: &str,
        dispatcher: Arc<Dispatcher>,
        pairing: Option<Arc<crate::pairing::PairingStore>>,
    ) -> Result<()> {
        let client = reqwest::Client::new();
        let url = open_socket_url(&client, app_token).await?;
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| AgentError::Tool(format!("slack connect: {e}")))?;
        eprintln!("[slack] socket-mode connected");
        let (mut sink, mut stream) = ws.split();
        let own_bot_id: Option<String> = None;
        while let Some(Ok(message)) = stream.next().await {
            let WsMessage::Text(text) = message else { continue };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else { continue };
            let envelope_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let envelope_id = envelope.get("envelope_id").and_then(|v| v.as_str()).unwrap_or("");
            if envelope_type == "hello" {
                continue;
            }
            // Ack every envelope promptly (Slack retries unacked ones).
            if !envelope_id.is_empty() {
                let ack = json!({"envelope_id": envelope_id});
                sink.send(WsMessage::Text(ack.to_string())).await.ok();
            }
            if envelope_type != "events_api" {
                continue;
            }
            let event = envelope.pointer("/payload/event").cloned().unwrap_or(json!({}));
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if event_type == "app_mention" || (event_type == "message" && event.get("subtype").is_none()) {
                if let Some(bot_id) = event.get("bot_id").and_then(|v| v.as_str()) {
                    if own_bot_id.as_deref() == Some(bot_id) || event.get("bot_profile").is_some() {
                        continue;
                    }
                }
                let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let has_files = event
                    .get("files")
                    .and_then(|v| v.as_array())
                    .map(|files| !files.is_empty())
                    .unwrap_or(false);
                if channel.is_empty() || (text.is_empty() && !has_files) {
                    continue;
                }
                // Strip a leading <@BOTID> mention so the agent sees clean text.
                let text = strip_mention(&text);
                // Slack file URLs are private — the bot bearer authenticates
                // the download (hermes slack adapter behavior).
                let attachments = {
                    let client = reqwest::Client::new();
                    let auth = format!("Bearer {bot_token}");
                    let mut downloaded = Vec::new();
                    if let Some(files) = event.get("files").and_then(|v| v.as_array()) {
                        for file in files {
                            let Some(url) = file.get("url_private").and_then(|v| v.as_str()) else {
                                continue;
                            };
                            let name = file.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let mime = file.get("mimetype").and_then(|v| v.as_str()).unwrap_or("");
                            let size = file.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                            if let Some(attachment) =
                                download_to_cache(&client, url, Some(&auth), mime, name, size).await
                            {
                                downloaded.push(attachment);
                            }
                        }
                    }
                    downloaded
                };
                let mut message_event = MessageEvent {
                    platform: "slack".into(),
                    chat_id: channel.clone(),
                    sender_id: event.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    sender_name: event.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    text,
                    message_id: event.get("ts").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    attachments,
                };
                // Plugin gate before auth (hermes ordering).
                if !pre_gateway_dispatch_gate(&mut message_event).await {
                    continue;
                }
                // Auth union: configured allowlist OR an approved pairing code.
                let authorized = allowlisted(&cfg.allowed_channel_ids, &channel)
                    || pairing
                        .as_ref()
                        .map(|store| store.is_approved("slack", &message_event.sender_id))
                        .unwrap_or(false);
                if !authorized {
                    eprintln!(
                        "[slack] refusing message from channel {channel} — add it to \
                         messaging.slack.allowed_channel_ids or approve a pairing code"
                    );
                    if let Some(store) = pairing.as_ref() {
                        if let Some(reply) = pairing_offer(store, "slack", &message_event.sender_id, &message_event.sender_name) {
                            post_message(bot_token, &channel, &reply).await;
                        }
                    }
                    continue;
                }
                let dispatcher = dispatcher.clone();
                let bot_token = bot_token.to_string();
                tokio::spawn(async move {
                    let outcome = match dispatcher.handle_event(message_event.clone()).await {
                        Ok(outcome) => outcome,
                        Err(e) => crate::messaging::DispatchOutcome {
                            reply: format!("error: {e}"),
                            transcript_echoes: Vec::new(),
                        },
                    };
                    for echo in &outcome.transcript_echoes {
                        post_message(&bot_token, &message_event.chat_id, echo).await;
                    }
                    let reply = outcome.reply;
                    // MEDIA:<path> tags become native Slack file uploads
                    // (hermes files.upload flow); the rest posts as text.
                    let (reply_text, media_paths) = extract_media_tags(&reply);
                    if !reply_text.trim().is_empty() {
                        post_message(&bot_token, &message_event.chat_id, &reply_text).await;
                    }
                    for path in media_paths {
                        upload_file(&bot_token, &message_event.chat_id, &path).await;
                    }
                });
            }
        }
        Ok(())
    }

    pub(crate) fn strip_mention(text: &str) -> String {
        let trimmed = text.trim_start();
        if trimmed.starts_with("<@") {
            if let Some(end) = trimmed.find('>') {
                return trimmed[end + 1..].trim().to_string();
            }
        }
        text.to_string()
    }

    /// chat.postMessage with Slack's ~40k limit chunked at 3500 chars.
    pub async fn post_message(bot_token: &str, channel: &str, text: &str) {
        let client = reqwest::Client::new();
        for chunk in chunk_text(text, 3500) {
            let result = client
                .post("https://slack.com/api/chat.postMessage")
                .header("Authorization", format!("Bearer {bot_token}"))
                .json(&json!({"channel": channel, "text": chunk}))
                .send()
                .await;
            match result {
                Ok(response) => {
                    if let Ok(value) = response.json::<Value>().await {
                        if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            eprintln!(
                                "[slack] postMessage failed: {}",
                                value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                            );
                        }
                    }
                }
                Err(e) => eprintln!("[slack] postMessage failed: {e}"),
            }
        }
    }

    /// Upload a local file as a native Slack file (MEDIA: delivery).
    /// Modern three-step flow: `files.getUploadURLExternal` → PUT the
    /// binary to the signed URL → `files.completeUploadExternal` into the
    /// channel (the legacy one-shot `files.upload` is retired).
    pub async fn upload_file(bot_token: &str, channel: &str, path: &std::path::Path) {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("[slack] cannot read {}: {e}", path.display());
                return;
            }
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let client = reqwest::Client::new();
        let auth = format!("Bearer {bot_token}");

        // Step 1: request an upload URL + file id.
        let url_response = client
            .get("https://slack.com/api/files.getUploadURLExternal")
            .header("Authorization", &auth)
            .query(&[("filename", &file_name), ("length", &data.len().to_string())])
            .send()
            .await;
        let url_value = match url_response {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("[slack] getUploadURLExternal parse failed: {e}");
                    return;
                }
            },
            Err(e) => {
                eprintln!("[slack] getUploadURLExternal failed: {e}");
                return;
            }
        };
        if !url_value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            eprintln!(
                "[slack] getUploadURLExternal failed: {}",
                url_value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
            );
            return;
        }
        let upload_url = match url_value.get("upload_url").and_then(|v| v.as_str()) {
            Some(url) => url.to_string(),
            None => {
                eprintln!("[slack] getUploadURLExternal returned no upload_url");
                return;
            }
        };
        let file_id = match url_value.get("file_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                eprintln!("[slack] getUploadURLExternal returned no file_id");
                return;
            }
        };

        // Step 2: PUT the raw bytes to the signed URL.
        let put_response = client
            .put(&upload_url)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await;
        match put_response {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                eprintln!("[slack] file upload PUT failed ({})", response.status());
                return;
            }
            Err(e) => {
                eprintln!("[slack] file upload PUT failed: {e}");
                return;
            }
        }

        // Step 3: complete the upload, sharing into the channel.
        let files_json = serde_json::to_string(&json!([{ "id": file_id, "title": file_name }]))
            .unwrap_or_default();
        let channels_json = serde_json::to_string(&json!([channel])).unwrap_or_default();
        let complete = client
            .post("https://slack.com/api/files.completeUploadExternal")
            .header("Authorization", &auth)
            .form(&[
                ("files", files_json.as_str()),
                ("channel_id", channel),
            ])
            .send()
            .await;
        match complete {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => {
                    if !value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        eprintln!(
                            "[slack] completeUploadExternal failed: {}",
                            value.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                        );
                    }
                    let _ = channels_json;
                }
                Err(e) => eprintln!("[slack] completeUploadExternal parse failed: {e}"),
            },
            Err(e) => eprintln!("[slack] completeUploadExternal failed: {e}"),
        }
    }
}

/// Split text into chunks no longer than `max` chars, preferring newline
/// boundaries (hermes message splitting).
pub(crate) fn chunk_text(text: &str, max: usize) -> Vec<String> {
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        // A single line longer than max must be hard-split.
        let mut rest = line;
        while rest.chars().count() > max {
            let split_at = rest
                .char_indices()
                .nth(max)
                .map(|(idx, _)| idx)
                .unwrap_or(rest.len());
            let (head, tail) = rest.split_at(split_at);
            out.push(head.to_string());
            rest = tail;
        }
        current.push_str(rest);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_disabled() {
        let cfg = MessagingConfig::default();
        assert!(!cfg.telegram.enabled && !cfg.discord.enabled && !cfg.slack.enabled);
    }

    #[test]
    fn allowlist_gate() {
        assert!(allowlisted(&["123".into()], "123"));
        assert!(!allowlisted(&["123".into()], "456"));
        assert!(!allowlisted(&[], "123"), "empty allowlist fails closed");
    }

    #[test]
    fn extract_media_tags_splits_paths_and_text() {
        let dir = std::env::temp_dir().join(format!(
            "ulnclaw-media-tags-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let media = dir.join("clip.mp4");
        std::fs::write(&media, b"fake").unwrap();

        // Existing path is extracted; missing paths stay as literal text.
        let reply = format!(
            "Here is your clip!\nMEDIA: {}\nMEDIA: /does/not/exist.png\nEnjoy!",
            media.display()
        );
        let (text, paths) = extract_media_tags(&reply);
        assert_eq!(paths, vec![media.clone()]);
        assert!(text.contains("Here is your clip!"));
        assert!(text.contains("Enjoy!"));
        assert!(text.contains("/does/not/exist.png"), "missing files stay literal");
        assert!(!text.contains(&format!("MEDIA: {}", media.display())));

        // Tilde expansion (restore HOME — env is process-global).
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.to_string_lossy().to_string());
        let home_media = dir.join("home.mp3");
        std::fs::write(&home_media, b"x").unwrap();
        let (text, paths) = extract_media_tags("MEDIA: ~/home.mp3");
        assert_eq!(paths, vec![home_media]);
        assert!(text.trim().is_empty());
        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }

        // No tags → unchanged.
        let (text, paths) = extract_media_tags("plain reply");
        assert!(paths.is_empty());
        assert_eq!(text, "plain reply");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_note_lists_paths_and_hints() {
        assert_eq!(attachment_note(&[]), "");
        let note = attachment_note(&[MediaAttachment {
            path: std::path::PathBuf::from("/tmp/cache/abc.jpg"),
            mime: "image/jpeg".into(),
            bytes: 1234,
            original_name: "pic.jpg".into(),
        }]);
        assert!(note.contains("/tmp/cache/abc.jpg"));
        assert!(note.contains("image/jpeg"));
        assert!(note.contains("1234 bytes"));
        assert!(note.contains("pic.jpg"));
        assert!(note.contains("vision_analyze"));
    }

    #[test]
    fn session_key_is_deterministic() {
        let event = MessageEvent {
            platform: "telegram".into(),
            chat_id: "-100123".into(),
            sender_id: "1".into(),
            sender_name: "u".into(),
            text: "hi".into(),
            message_id: "m".into(),
            attachments: Vec::new(),
        };
        assert_eq!(Dispatcher::session_key(&event), "platform-telegram--100123");
    }

    #[test]
    fn chunk_short_text_unchanged() {
        assert_eq!(chunk_text("hello", 10), vec!["hello".to_string()]);
    }

    #[test]
    fn chunk_prefers_newlines() {
        let text = "aaaa\nbbbb\ncccc";
        let chunks = chunk_text(text, 10);
        assert!(chunks.iter().all(|c| c.chars().count() <= 10));
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn chunk_hard_splits_long_lines() {
        let text = "x".repeat(25);
        let chunks = chunk_text(&text, 10);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 10));
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn slack_mention_stripped() {
        assert_eq!(slack::strip_mention("<@U123> hello"), "hello");
        assert_eq!(slack::strip_mention("plain text"), "plain text");
    }

    #[test]
    fn token_resolution_prefers_config() {
        assert_eq!(resolve_token("abc", "ULNCLAW_NEVER_SET_XYZ").as_deref(), Some("abc"));
        assert_eq!(resolve_token("  ", "ULNCLAW_NEVER_SET_XYZ"), None);
    }
}
