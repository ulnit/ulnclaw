//! Buzz platform adapter — port of hermes `plugins/platforms/buzz`
//! @ v2026.8.3 (adapter.py, CLI poll transport).
//!
//! Buzz is Block's Nostr-based human+agent collaboration platform. The
//! hermes adapter shells out to the `buzz` CLI ("JSON in, JSON out");
//! this port keeps that contract with `tokio::process`: inbound polls
//! run `buzz messages get --channel <id> --limit 50 [--since <ts>]`
//! every 4 s (hermes defaults), outbound rides
//! `buzz messages send --channel <id> --content -` with the body on
//! stdin.
//!
//! Intake mirrors hermes: only Nostr kind-9 chat events dispatch,
//! event ids dedup per channel (500-entry cap), own-pubkey echoes are
//! suppressed (`BUZZ_PUBKEY`), channels are require-mention gated
//! (leading `@mention` stripped before dispatch) while DMs always
//! pass, and an optional pubkey allowlist filters senders. The NIP-42
//! WebSocket transport and npub/hex bech32 conversion are not ported
//! (hex pubkeys only; the WS mode needs a relay auth dance).

use crate::messaging::{Dispatcher, MessageEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

/// hermes `_CHAT_KIND` — Buzz chat messages are Nostr kind 9.
const CHAT_KIND: u64 = 9;
/// hermes `_FETCH_LIMIT`.
const FETCH_LIMIT: u32 = 50;
/// hermes `_SEEN_CAP`.
const SEEN_CAP: usize = 500;
/// hermes `_DEFAULT_POLL_INTERVAL` / `_MIN_POLL_INTERVAL`.
const DEFAULT_POLL_INTERVAL_MS: u64 = 4000;
const MIN_POLL_INTERVAL_MS: u64 = 1000;
/// hermes `_CLI_TIMEOUT`.
const CLI_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MESSAGE_LENGTH: usize = 4000;

/// `[messaging.buzz]` — Buzz adapter (hermes `platforms.buzz` plugin
/// config + `BUZZ_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuzzConfig {
    pub enabled: bool,
    /// Path to the buzz CLI binary (fallback `BUZZ_CLI`, default
    /// `buzz`).
    pub cli_path: String,
    /// Channel ids to watch (fallback `BUZZ_CHANNELS`).
    pub channels: Vec<String>,
    /// Own hex pubkey for echo suppression (fallback `BUZZ_PUBKEY`).
    pub self_pubkey: String,
    /// Sender pubkeys allowed to talk to the bot (fallback
    /// `BUZZ_ALLOWED_USERS`). Empty = allow all.
    pub allowed_users: Vec<String>,
    /// Require an @mention in channels (hermes default true).
    pub require_mention: bool,
    /// Poll interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Cron/notification delivery channel (fallback `BUZZ_HOME_CHANNEL`).
    pub home_channel: String,
}

impl Default for BuzzConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cli_path: String::new(),
            channels: Vec::new(),
            self_pubkey: String::new(),
            allowed_users: Vec::new(),
            require_mention: true,
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
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
pub struct ResolvedBuzz {
    pub cli_path: String,
    pub channels: Vec<String>,
    pub self_pubkey: String,
    pub allowed_users: Vec<String>,
    pub require_mention: bool,
    pub poll_interval_ms: u64,
    pub home_channel: String,
}

impl BuzzConfig {
    pub fn resolve(&self) -> ResolvedBuzz {
        ResolvedBuzz {
            cli_path: env_trim("BUZZ_CLI").unwrap_or_else(|| {
                if self.cli_path.is_empty() {
                    "buzz".to_string()
                } else {
                    self.cli_path.clone()
                }
            }),
            channels: env_list("BUZZ_CHANNELS").unwrap_or_else(|| self.channels.clone()),
            self_pubkey: env_trim("BUZZ_PUBKEY")
                .unwrap_or_else(|| self.self_pubkey.clone())
                .to_lowercase(),
            allowed_users: env_list("BUZZ_ALLOWED_USERS")
                .unwrap_or_else(|| self.allowed_users.clone())
                .into_iter()
                .map(|u| u.to_lowercase())
                .collect(),
            require_mention: env_trim("BUZZ_REQUIRE_MENTION")
                .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no"))
                .unwrap_or(self.require_mention),
            poll_interval_ms: env_trim("BUZZ_POLL_INTERVAL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(self.poll_interval_ms)
                .max(MIN_POLL_INTERVAL_MS),
            home_channel: env_trim("BUZZ_HOME_CHANNEL")
                .unwrap_or_else(|| self.home_channel.clone()),
        }
    }
}

/// hermes `_parse_json_list` — tolerant JSON array parse.
pub fn parse_json_list(out: &str) -> Vec<Value> {
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Vec::new())
}

/// Per-channel poll state (hermes `_channel_state`).
struct ChannelState {
    seen: VecDeque<String>,
    seen_set: HashSet<String>,
    last_ts: i64,
}

impl ChannelState {
    fn new() -> Self {
        Self {
            seen: VecDeque::new(),
            seen_set: HashSet::new(),
            last_ts: 0,
        }
    }

    /// Returns true when the id is new (records it).
    fn remember(&mut self, id: &str) -> bool {
        if self.seen_set.contains(id) {
            return false;
        }
        self.seen_set.insert(id.to_string());
        self.seen.push_back(id.to_string());
        while self.seen.len() > SEEN_CAP {
            if let Some(old) = self.seen.pop_front() {
                self.seen_set.remove(&old);
            }
        }
        true
    }
}

/// hermes mention detection: content carries `@<name>` or our pubkey.
pub fn is_mentioned(content: &str, self_pubkey: &str) -> bool {
    if !self_pubkey.is_empty() {
        let lower = content.to_lowercase();
        if lower.contains(&self_pubkey.to_lowercase()) {
            return true;
        }
        // buzz UI shows truncated pubkeys — accept a prefix of at least
        // MIN_PUBKEY_PREFIX chars as a mention.
        const MIN_PUBKEY_PREFIX: usize = 6;
        if self_pubkey.len() > MIN_PUBKEY_PREFIX {
            for end in (MIN_PUBKEY_PREFIX..self_pubkey.len()).rev() {
                if lower.contains(&self_pubkey[..end].to_lowercase()) {
                    return true;
                }
            }
        }
    }
    content.contains('@')
}

/// Strip a leading `@mention ` token (hermes `_strip_mention`).
pub fn strip_mention(content: &str) -> String {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix('@') {
        if let Some(space) = rest.find(char::is_whitespace) {
            return rest[space..].trim().to_string();
        }
    }
    content.to_string()
}

/// Run one buzz CLI invocation, returning (exit_code, stdout).
async fn run_cli(cli: &str, args: &[&str], stdin_body: Option<&str>) -> Result<(i32, String), String> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;
    let mut cmd = Command::new(cli);
    cmd.args(args)
        .stdin(if stdin_body.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn {cli}: {e}"))?;
    if let Some(body) = stdin_body {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(body.as_bytes()).await;
        }
    }
    let output = tokio::time::timeout(CLI_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("buzz CLI timed out after {}s", CLI_TIMEOUT.as_secs()))?
        .map_err(|e| format!("buzz CLI wait: {e}"))?;
    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
    ))
}

/// Entry point spawned by `run_messaging`.
pub async fn run(
    cfg: BuzzConfig,
    dispatcher: Arc<Dispatcher>,
    pairing: Option<Arc<crate::pairing::PairingStore>>,
) {
    let resolved = cfg.resolve();
    if resolved.channels.is_empty() {
        eprintln!(
            "[buzz] disabled: no channels configured (set [messaging.buzz] channels or BUZZ_CHANNELS)"
        );
        return;
    }
    crate::messaging::register_platform_sender(
        "buzz",
        Arc::new(BuzzSender {
            cfg: resolved.clone(),
        }),
    );
    let mut states: HashMap<String, ChannelState> = HashMap::new();
    for channel in &resolved.channels {
        states.insert(channel.clone(), ChannelState::new());
    }
    eprintln!(
        "[buzz] polling {} channel(s) via {} every {}ms",
        resolved.channels.len(),
        resolved.cli_path,
        resolved.poll_interval_ms
    );
    loop {
        for channel in resolved.channels.clone() {
            poll_channel(&resolved, &dispatcher, &pairing, &channel, states.get_mut(&channel).unwrap())
                .await;
        }
        tokio::time::sleep(Duration::from_millis(resolved.poll_interval_ms)).await;
    }
}

/// hermes `_poll_channel` + `_handle_event`.
async fn poll_channel(
    cfg: &ResolvedBuzz,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
    channel_id: &str,
    state: &mut ChannelState,
) {
    let since = state.last_ts.to_string();
    let limit_str = FETCH_LIMIT.to_string();
    let mut args: Vec<&str> = vec!["messages", "get", "--channel", channel_id, "--limit", &limit_str];
    if state.last_ts > 0 {
        args.push("--since");
        args.push(&since);
    }
    let (code, out) = match run_cli(&cfg.cli_path, &args, None).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[buzz] poll {channel_id}: {e}");
            return;
        }
    };
    if code != 0 {
        return;
    }
    for event in parse_json_list(&out) {
        handle_event(cfg, dispatcher, pairing, channel_id, state, &event).await;
    }
}

async fn handle_event(
    cfg: &ResolvedBuzz,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
    channel_id: &str,
    state: &mut ChannelState,
    event: &Value,
) {
    let event_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let created_at = event.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);
    if event_id.is_empty() || !state.remember(&event_id) {
        return;
    }
    state.last_ts = state.last_ts.max(created_at);
    if event.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) != CHAT_KIND {
        return;
    }
    let pubkey = event
        .get("pubkey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let content = event
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if pubkey.is_empty() || content.is_empty() {
        return;
    }
    if !cfg.self_pubkey.is_empty() && pubkey == cfg.self_pubkey {
        return;
    }
    // Channel mention gate (DMs are channels configured as DMs by the
    // operator via a `dm:` prefix in BUZZ_CHANNELS — hermes latches DMs
    // dynamically; the port uses explicit configuration).
    let is_dm = channel_id.starts_with("dm:");
    if !is_dm && cfg.require_mention && !is_mentioned(&content, &cfg.self_pubkey) {
        return;
    }
    if !cfg.allowed_users.is_empty() && !cfg.allowed_users.iter().any(|u| u == &pubkey || u == "*")
    {
        if let Some(store) = pairing {
            if !store.is_approved("buzz", &pubkey) {
                if let Some(code_msg) =
                    crate::messaging::pairing_offer_public(store, "buzz", &pubkey, &pubkey)
                {
                    let _ = send_via_cli(cfg, channel_id, &code_msg).await;
                }
                return;
            }
        } else {
            return;
        }
    }
    let dispatch_text = strip_mention(&content);
    let chat_id = if is_dm {
        channel_id.trim_start_matches("dm:").to_string()
    } else {
        channel_id.to_string()
    };
    let mut gate_event = MessageEvent {
        platform: "buzz".into(),
        chat_id,
        sender_id: pubkey.clone(),
        sender_name: pubkey[..pubkey.len().min(8)].to_string(),
        text: dispatch_text,
        message_id: event_id,
        attachments: Vec::new(),
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
    if !reply_text.is_empty() {
        if let Err(e) = send_via_cli(cfg, channel_id, &reply_text).await {
            eprintln!("[buzz] reply to {channel_id} failed: {e}");
        }
    }
}

/// hermes send — `buzz messages send --channel <id> --content -`.
pub async fn send_via_cli(cfg: &ResolvedBuzz, channel_id: &str, content: &str) -> Result<(), String> {
    let body: String = content.chars().take(MAX_MESSAGE_LENGTH).collect();
    let (code, out) = run_cli(
        &cfg.cli_path,
        &["messages", "send", "--channel", channel_id, "--content", "-"],
        Some(&body),
    )
    .await?;
    if code != 0 {
        return Err(format!("buzz send exit {code}: {}", out.trim()));
    }
    Ok(())
}

struct BuzzSender {
    cfg: ResolvedBuzz,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for BuzzSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        if let Err(e) = send_via_cli(&self.cfg, chat_id, text).await {
            eprintln!("[buzz] send_text to {chat_id} failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_list_tolerant() {
        let events = parse_json_list(r#"[{"id":"a"},{"id":"b"}]"#);
        assert_eq!(events.len(), 2);
        assert!(parse_json_list("").is_empty());
        assert!(parse_json_list("not json").is_empty());
        assert!(parse_json_list("{}").is_empty());
    }

    #[test]
    fn seen_cap_bounds_dedup_set() {
        let mut state = ChannelState::new();
        for i in 0..SEEN_CAP + 100 {
            assert!(state.remember(&format!("evt{i}")));
        }
        assert_eq!(state.seen.len(), SEEN_CAP);
        // Oldest evicted — can be re-seen.
        assert!(state.remember("evt0"));
        // Recent still dedups.
        assert!(!state.remember(&format!("evt{}", SEEN_CAP + 99)));
    }

    #[test]
    fn mention_detection_and_strip() {
        assert!(is_mentioned("hey @chip help", ""));
        assert!(is_mentioned(
            "hello abc123",
            "abc123def456"
        ));
        assert!(!is_mentioned("plain message", "abc123def456"));
        assert_eq!(strip_mention("@Chip /whoami"), "/whoami");
        assert_eq!(strip_mention("no mention"), "no mention");
        assert_eq!(strip_mention("@solo"), "@solo");
    }

    #[test]
    fn kind_filter() {
        let event: Value = serde_json::from_str(r#"{"kind":9,"id":"x"}"#).unwrap();
        assert_eq!(event.get("kind").and_then(|v| v.as_u64()), Some(CHAT_KIND));
        let other: Value = serde_json::from_str(r#"{"kind":44100}"#).unwrap();
        assert_ne!(other.get("kind").and_then(|v| v.as_u64()), Some(CHAT_KIND));
    }

    #[test]
    fn resolve_defaults() {
        let _guard = crate::models_dev::test_env_lock();
        let resolved = BuzzConfig::default().resolve();
        assert_eq!(resolved.cli_path, "buzz");
        assert!(resolved.require_mention);
        assert_eq!(resolved.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);
    }

    #[test]
    fn resolve_env_overrides() {
        let _guard = crate::models_dev::test_env_lock();
        std::env::set_var("BUZZ_CHANNELS", "chan1, chan2");
        std::env::set_var("BUZZ_REQUIRE_MENTION", "false");
        std::env::set_var("BUZZ_POLL_INTERVAL_MS", "200");
        let resolved = BuzzConfig::default().resolve();
        assert_eq!(resolved.channels, vec!["chan1".to_string(), "chan2".to_string()]);
        assert!(!resolved.require_mention);
        assert_eq!(resolved.poll_interval_ms, MIN_POLL_INTERVAL_MS); // floor
        std::env::remove_var("BUZZ_CHANNELS");
        std::env::remove_var("BUZZ_REQUIRE_MENTION");
        std::env::remove_var("BUZZ_POLL_INTERVAL_MS");
    }

    #[test]
    fn dm_prefix_classification() {
        assert!("dm:abc123".starts_with("dm:"));
        assert_eq!("dm:abc123".trim_start_matches("dm:"), "abc123");
        assert!(!"general".starts_with("dm:"));
    }

    #[test]
    fn event_field_extraction() {
        let event: Value = serde_json::from_str(
            r#"{"id":"deadbeef","kind":9,"pubkey":"AB12","content":" hello ","created_at":1700000000}"#,
        )
        .unwrap();
        assert_eq!(event.get("id").and_then(|v| v.as_str()), Some("deadbeef"));
        assert_eq!(
            event.get("pubkey").and_then(|v| v.as_str()).unwrap().to_lowercase(),
            "ab12"
        );
        assert_eq!(event.get("created_at").and_then(|v| v.as_i64()), Some(1700000000));
    }
}
