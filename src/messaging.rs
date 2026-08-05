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

/// Normalized incoming message (hermes `MessageEvent`, core fields).
#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub platform: String,
    pub chat_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
    pub message_id: String,
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

    /// Run one agent turn for the event's chat; returns the reply text.
    pub async fn handle_event(self: &Arc<Self>, event: MessageEvent) -> Result<String> {
        let key = Self::session_key(&event);
        {
            let mut busy = self.busy.lock().await;
            if *busy.get(&key).unwrap_or(&false) {
                return Ok("(the previous message is still being processed — please wait)".into());
            }
            busy.insert(key.clone(), true);
        }
        let result = self.run_turn(&key, &event).await;
        self.busy.lock().await.insert(key, false);
        result
    }

    async fn run_turn(&self, key: &str, event: &MessageEvent) -> Result<String> {
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
        let prompt = if event.sender_name.is_empty() {
            event.text.clone()
        } else {
            format!("{}: {}", event.sender_name, event.text)
        };
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
        Ok(result.content)
    }
}

/// Access gate (hermes pairing): allowlisted ids pass; everything else
/// is refused and reported so the user knows what to allowlist.
fn allowlisted(allowlist: &[String], id: &str) -> bool {
    allowlist.iter().any(|allowed| allowed == id)
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
    if msg.telegram.enabled {
        let cfg = msg.telegram.clone();
        let dispatcher = dispatcher.clone();
        tasks.push(tokio::spawn(async move {
            telegram::run(cfg, dispatcher).await;
        }));
    }
    if msg.discord.enabled {
        let cfg = msg.discord.clone();
        let dispatcher = dispatcher.clone();
        tasks.push(tokio::spawn(async move {
            discord::run(cfg, dispatcher).await;
        }));
    }
    if msg.slack.enabled {
        let cfg = msg.slack.clone();
        let dispatcher = dispatcher.clone();
        tasks.push(tokio::spawn(async move {
            slack::run(cfg, dispatcher).await;
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

// ---------------------------------------------------------------------------
// Telegram — Bot API long-polling
// ---------------------------------------------------------------------------

pub mod telegram {
    use super::*;

    const API: &str = "https://api.telegram.org";

    pub async fn run(cfg: TelegramConfig, dispatcher: Arc<Dispatcher>) {
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
                let Some(text) = message.get("text").and_then(|v| v.as_str()) else { continue };
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
                };
                // Plugin gate before auth (hermes ordering).
                if !pre_gateway_dispatch_gate(&mut event).await {
                    continue;
                }
                if !allowlisted(&cfg.allowed_chat_ids, &chat_id) {
                    eprintln!(
                        "[telegram] refusing message from chat {chat_id} — add it to \
                         messaging.telegram.allowed_chat_ids"
                    );
                    continue;
                }
                let dispatcher = dispatcher.clone();
                let client = client.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    let reply = match dispatcher.handle_event(event.clone()).await {
                        Ok(text) => text,
                        Err(e) => format!("error: {e}"),
                    };
                    send_message(&client, &token, &event.chat_id, &reply).await;
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

    pub async fn run(cfg: DiscordConfig, dispatcher: Arc<Dispatcher>) {
        let Some(token) = resolve_token(&cfg.bot_token, "DISCORD_BOT_TOKEN") else {
            eprintln!("[discord] disabled: no bot_token configured (set messaging.discord.bot_token or DISCORD_BOT_TOKEN)");
            return;
        };
        loop {
            if let Err(e) = run_session(&cfg, &token, dispatcher.clone()).await {
                eprintln!("[discord] gateway session ended: {e} — reconnecting in 5s");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn run_session(cfg: &DiscordConfig, token: &str, dispatcher: Arc<Dispatcher>) -> Result<()> {
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
                                handle_message_create(cfg, token, &dispatcher, payload.get("d").cloned().unwrap_or(json!({}))).await;
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

    async fn handle_message_create(cfg: &DiscordConfig, token: &str, dispatcher: &Arc<Dispatcher>, data: Value) {
        // Ignore bot/webhook messages (never talk to ourselves).
        if data.get("author").and_then(|a| a.get("bot")).and_then(|v| v.as_bool()).unwrap_or(false) {
            return;
        }
        let Some(text) = data.get("content").and_then(|v| v.as_str()).filter(|t| !t.is_empty()) else {
            return;
        };
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
        };
        // Plugin gate before auth (hermes ordering).
        if !pre_gateway_dispatch_gate(&mut event).await {
            return;
        }
        if !allowlisted(&cfg.allowed_channel_ids, &channel_id) {
            eprintln!(
                "[discord] refusing message from channel {channel_id} — add it to \
                 messaging.discord.allowed_channel_ids"
            );
            return;
        }
        let dispatcher = dispatcher.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            let reply = match dispatcher.handle_event(event.clone()).await {
                Ok(text) => text,
                Err(e) => format!("error: {e}"),
            };
            send_channel_message(&token, &event.chat_id, &reply).await;
        });
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

    pub async fn run(cfg: SlackConfig, dispatcher: Arc<Dispatcher>) {
        let Some(bot_token) = resolve_token(&cfg.bot_token, "SLACK_BOT_TOKEN") else {
            eprintln!("[slack] disabled: no bot_token configured (set messaging.slack.bot_token or SLACK_BOT_TOKEN)");
            return;
        };
        let Some(app_token) = resolve_token(&cfg.app_token, "SLACK_APP_TOKEN") else {
            eprintln!("[slack] disabled: no app_token configured (set messaging.slack.app_token or SLACK_APP_TOKEN)");
            return;
        };
        loop {
            match run_socket_session(&cfg, &bot_token, &app_token, dispatcher.clone()).await {
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
                if channel.is_empty() || text.is_empty() {
                    continue;
                }
                // Strip a leading <@BOTID> mention so the agent sees clean text.
                let text = strip_mention(&text);
                let mut message_event = MessageEvent {
                    platform: "slack".into(),
                    chat_id: channel.clone(),
                    sender_id: event.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    sender_name: event.get("user").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    text,
                    message_id: event.get("ts").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                };
                // Plugin gate before auth (hermes ordering).
                if !pre_gateway_dispatch_gate(&mut message_event).await {
                    continue;
                }
                if !allowlisted(&cfg.allowed_channel_ids, &channel) {
                    eprintln!(
                        "[slack] refusing message from channel {channel} — add it to \
                         messaging.slack.allowed_channel_ids"
                    );
                    continue;
                }
                let dispatcher = dispatcher.clone();
                let bot_token = bot_token.to_string();
                tokio::spawn(async move {
                    let reply = match dispatcher.handle_event(message_event.clone()).await {
                        Ok(text) => text,
                        Err(e) => format!("error: {e}"),
                    };
                    post_message(&bot_token, &message_event.chat_id, &reply).await;
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
}

/// Split text into chunks no longer than `max` chars, preferring newline
/// boundaries (hermes message splitting).
fn chunk_text(text: &str, max: usize) -> Vec<String> {
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
    fn session_key_is_deterministic() {
        let event = MessageEvent {
            platform: "telegram".into(),
            chat_id: "-100123".into(),
            sender_id: "1".into(),
            sender_name: "u".into(),
            text: "hi".into(),
            message_id: "m".into(),
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
