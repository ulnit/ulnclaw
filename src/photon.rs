//! Photon Spectrum (iMessage) platform adapter — port of hermes
//! `plugins/platforms/photon` @ v2026.8.3 (adapter.py, sidecar-client
//! transport).
//!
//! In hermes both directions flow through a supervised Node sidecar
//! running the TypeScript-only `spectrum-ts` SDK: inbound is an NDJSON
//! stream from `GET /inbound`, outbound posts to `/send`
//! (`{spaceId, text, format?}`), `/send-richlink`, and
//! `/send-attachment`, health via `/healthz`. This port keeps the wire
//! protocol but treats the sidecar as an external service — ulnclaw
//! does not spawn or npm-install the Node sidecar (documented
//! divergence; run the hermes sidecar and point
//! `[messaging.photon] sidecar_url` at it).
//!
//! Intake: NDJSON events with `type == "message"` dispatch (sender
//! dedup by messageId, 300 s window), allowlist ∪ pairing gate, media
//! attachments (sidecar-provided local paths) into the media cache.
//! Replies chunk at the hermes cap and post to `/send` with the shared
//! bearer token.

use crate::messaging::{Dispatcher, MediaAttachment, MessageEvent};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_MESSAGE_LENGTH: usize = 4000;
const DEDUP_WINDOW_SECS: u64 = 300;
const DEDUP_MAX_SIZE: usize = 1000;
const API_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// `[messaging.photon]` — Photon adapter (hermes `platforms.photon`
/// plugin config + `PHOTON_*` env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotonConfig {
    pub enabled: bool,
    /// Sidecar base URL (fallback `PHOTON_SIDECAR_URL`, default
    /// `http://127.0.0.1:7370`).
    pub sidecar_url: String,
    /// Shared bearer token (fallback `PHOTON_SIDECAR_TOKEN`).
    pub token: String,
    /// Sender ids allowed to talk to the bot (fallback
    /// `PHOTON_ALLOWED_USERS`).
    pub allowed_users: Vec<String>,
    pub allow_all_users: bool,
    /// Cron/notification delivery space (fallback `PHOTON_HOME_CHANNEL`).
    pub home_channel: String,
}

impl Default for PhotonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sidecar_url: String::new(),
            token: String::new(),
            allowed_users: Vec::new(),
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
pub struct ResolvedPhoton {
    pub sidecar_url: String,
    pub token: String,
    pub allowed_users: Vec<String>,
    pub allow_all_users: bool,
    pub home_channel: String,
}

impl PhotonConfig {
    pub fn resolve(&self) -> ResolvedPhoton {
        ResolvedPhoton {
            sidecar_url: env_trim("PHOTON_SIDECAR_URL")
                .unwrap_or_else(|| self.sidecar_url.clone())
                .trim_end_matches('/')
                .to_string(),
            token: env_trim("PHOTON_SIDECAR_TOKEN").unwrap_or_else(|| self.token.clone()),
            allowed_users: env_list("PHOTON_ALLOWED_USERS")
                .unwrap_or_else(|| self.allowed_users.clone()),
            allow_all_users: env_trim("PHOTON_ALLOW_ALL_USERS")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(self.allow_all_users),
            home_channel: env_trim("PHOTON_HOME_CHANNEL")
                .unwrap_or_else(|| self.home_channel.clone()),
        }
    }
}

struct Runtime {
    cfg: ResolvedPhoton,
    client: reqwest::Client,
    /// messageId dedup (gRPC stream is at-least-once).
    seen: Mutex<HashMap<String, u64>>,
}

impl Runtime {
    async fn is_duplicate(&self, message_id: &str) -> bool {
        if message_id.is_empty() {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut seen = self.seen.lock().await;
        seen.retain(|_, ts| now.saturating_sub(*ts) < DEDUP_WINDOW_SECS);
        if seen.contains_key(message_id) {
            return true;
        }
        if seen.len() >= DEDUP_MAX_SIZE {
            let mut entries: Vec<(String, u64)> = seen.drain().collect();
            entries.sort_by_key(|(_, ts)| *ts);
            entries.truncate(DEDUP_MAX_SIZE / 2);
            *seen = entries.into_iter().collect();
        }
        seen.insert(message_id.to_string(), now);
        false
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.cfg.token.is_empty() {
            req
        } else {
            req.bearer_auth(&self.cfg.token)
        }
    }
}

/// Entry point spawned by `run_messaging`.
pub async fn run(
    cfg: PhotonConfig,
    dispatcher: Arc<Dispatcher>,
    pairing: Option<Arc<crate::pairing::PairingStore>>,
) {
    let mut resolved = cfg.resolve();
    if resolved.sidecar_url.is_empty() {
        resolved.sidecar_url = "http://127.0.0.1:7370".to_string();
    }
    let runtime = Arc::new(Runtime {
        client: reqwest::Client::new(),
        cfg: resolved,
        seen: Mutex::new(HashMap::new()),
    });
    crate::messaging::register_platform_sender(
        "photon",
        Arc::new(PhotonSender {
            runtime: runtime.clone(),
        }),
    );
    loop {
        // Health gate (hermes /healthz wait).
        match wait_for_sidecar(&runtime).await {
            Ok(()) => {}
            Err(e) => {
                eprintln!("[photon] sidecar wait failed: {e} — retrying");
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        }
        match consume_inbound(&runtime, &dispatcher, &pairing).await {
            Ok(()) => {}
            Err(e) => eprintln!("[photon] inbound stream error: {e}"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn wait_for_sidecar(runtime: &Arc<Runtime>) -> Result<(), String> {
    let url = format!("{}/healthz", runtime.cfg.sidecar_url);
    let resp = runtime
        .authed(runtime.client.get(&url))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("healthz: {e}"))?;
    if resp.status().is_success() {
        eprintln!("[photon] sidecar healthy at {}", runtime.cfg.sidecar_url);
        Ok(())
    } else {
        Err(format!("healthz HTTP {}", resp.status()))
    }
}

/// hermes inbound consumer — NDJSON stream from `GET /inbound`.
async fn consume_inbound(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
) -> Result<(), String> {
    let url = format!("{}/inbound", runtime.cfg.sidecar_url);
    let resp = runtime
        .authed(runtime.client.get(&url))
        .send()
        .await
        .map_err(|e| format!("inbound connect: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("inbound HTTP {}", resp.status()));
    }
    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("inbound read: {e}"))?;
        buffer.extend_from_slice(&chunk);
        while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buffer.drain(..pos + 1).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            if line.is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            handle_inbound(runtime, dispatcher, pairing, &event).await;
        }
    }
    Ok(())
}

/// hermes `_dispatch_inbound` — message events only.
async fn handle_inbound(
    runtime: &Arc<Runtime>,
    dispatcher: &Arc<Dispatcher>,
    pairing: &Option<Arc<crate::pairing::PairingStore>>,
    event: &Value,
) {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if event_type != "message" {
        return;
    }
    let message_id = event
        .get("messageId")
        .or_else(|| event.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if runtime.is_duplicate(&message_id).await {
        return;
    }
    let space_id = event
        .get("spaceId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_id = event
        .get("senderId")
        .or_else(|| event.get("sender"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if space_id.is_empty() || sender_id.is_empty() {
        return;
    }
    let text = event
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    // Allowlist ∪ pairing.
    if !runtime.cfg.allow_all_users
        && !runtime.cfg.allowed_users.iter().any(|u| u == &sender_id || u == "*")
    {
        if let Some(store) = pairing {
            if !store.is_approved("photon", &sender_id) {
                if let Some(code_msg) = crate::messaging::pairing_offer_public(
                    store, "photon", &sender_id, &sender_id,
                ) {
                    let _ = send_text(runtime, &space_id, &code_msg).await;
                }
                return;
            }
        } else {
            eprintln!("[photon] unauthorized sender {sender_id} — add to allowed_users");
            return;
        }
    }
    // Attachments: sidecar-downloaded local paths.
    let mut attachments = Vec::new();
    if let Some(paths) = event.get("attachments").and_then(|v| v.as_array()) {
        for path_value in paths {
            let Some(path) = path_value.as_str() else { continue };
            if !std::path::Path::new(path).is_absolute() {
                continue;
            }
            match tokio::fs::read(path).await {
                Ok(bytes) => {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
                    let mime = match ext.as_str() {
                        "jpg" | "jpeg" | "png" | "gif" | "webp" => format!("image/{ext}"),
                        "mp3" | "wav" | "ogg" | "m4a" | "aac" => format!("audio/{ext}"),
                        "mp4" | "mov" => format!("video/{ext}"),
                        _ => "application/octet-stream".to_string(),
                    };
                    match crate::media_cache::cache_media_bytes(
                        &crate::config::ulnclaw_home(),
                        &bytes,
                        &mime,
                        &name,
                    ) {
                        Ok(cached) => attachments.push(MediaAttachment {
                            path: cached,
                            mime: mime.clone(),
                            bytes: bytes.len() as u64,
                            original_name: name,
                        }),
                        Err(e) => eprintln!("[photon] media cache failed: {e}"),
                    }
                }
                Err(e) => eprintln!("[photon] attachment unreadable ({path}): {e}"),
            }
        }
    }
    if text.is_empty() && attachments.is_empty() {
        return;
    }
    let mut gate_event = MessageEvent {
        platform: "photon".into(),
        chat_id: space_id.clone(),
        sender_id: sender_id.clone(),
        sender_name: sender_id,
        text: if text.is_empty() {
            "[media message]".to_string()
        } else {
            text
        },
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
    let (reply_text, media_paths) = crate::messaging::extract_media_tags(&full);
    for path in &media_paths {
        send_attachment(runtime, &space_id, path).await;
    }
    let reply_text = reply_text.trim().to_string();
    if !reply_text.is_empty() {
        if let Err(e) = send_text(runtime, &space_id, &reply_text).await {
            eprintln!("[photon] reply to {space_id} failed: {e}");
        }
    }
}

/// hermes `/send` — `{spaceId, text}` (+ markdown format hint).
async fn send_text(runtime: &Runtime, space_id: &str, content: &str) -> Result<(), String> {
    let url = format!("{}/send", runtime.cfg.sidecar_url);
    let body: String = content.chars().take(MAX_MESSAGE_LENGTH).collect();
    let payload = json!({ "spaceId": space_id, "text": body, "format": "markdown" });
    let resp = runtime
        .authed(runtime.client.post(&url))
        .timeout(API_TIMEOUT)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        Err(format!("HTTP {status}: {}", &err[..err.len().min(200)]))
    }
}

/// hermes `/send-attachment` — `{spaceId, filePath}`.
async fn send_attachment(runtime: &Runtime, space_id: &str, path: &std::path::Path) {
    let url = format!("{}/send-attachment", runtime.cfg.sidecar_url);
    let payload = json!({ "spaceId": space_id, "filePath": path.to_string_lossy() });
    let result = runtime
        .authed(runtime.client.post(&url))
        .timeout(API_TIMEOUT)
        .json(&payload)
        .send()
        .await;
    match result {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => eprintln!("[photon] send-attachment HTTP {}", resp.status()),
        Err(e) => eprintln!("[photon] send-attachment failed: {e}"),
    }
}

struct PhotonSender {
    runtime: Arc<Runtime>,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for PhotonSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        if let Err(e) = send_text(&self.runtime, chat_id, text).await {
            eprintln!("[photon] send_text to {chat_id} failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_and_env() {
        let _guard = crate::models_dev::test_env_lock();
        let cfg = PhotonConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(resolved.sidecar_url, "");
        assert!(!resolved.allow_all_users);

        std::env::set_var("PHOTON_SIDECAR_URL", "http://10.0.0.9:7370/");
        std::env::set_var("PHOTON_ALLOWED_USERS", "alice, bob");
        let resolved = cfg.resolve();
        assert_eq!(resolved.sidecar_url, "http://10.0.0.9:7370");
        assert_eq!(
            resolved.allowed_users,
            vec!["alice".to_string(), "bob".to_string()]
        );
        std::env::remove_var("PHOTON_SIDECAR_URL");
        std::env::remove_var("PHOTON_ALLOWED_USERS");
    }

    #[tokio::test]
    async fn dedup_window() {
        let runtime = Runtime {
            client: reqwest::Client::new(),
            cfg: PhotonConfig::default().resolve(),
            seen: Mutex::new(HashMap::new()),
        };
        assert!(!runtime.is_duplicate("m1").await);
        assert!(runtime.is_duplicate("m1").await);
        assert!(!runtime.is_duplicate("").await);
    }

    #[test]
    fn inbound_event_filters() {
        let msg: Value = serde_json::from_str(
            r#"{"type":"message","messageId":"x","spaceId":"s","senderId":"u","text":"hi"}"#,
        )
        .unwrap();
        assert_eq!(msg.get("type").and_then(|v| v.as_str()), Some("message"));
        let other: Value = serde_json::from_str(r#"{"type":"poll_option"}"#).unwrap();
        assert_ne!(other.get("type").and_then(|v| v.as_str()), Some("message"));
    }

    #[test]
    fn send_payload_shape() {
        let payload = json!({ "spaceId": "s1", "text": "hello", "format": "markdown" });
        assert_eq!(payload["spaceId"], "s1");
        assert_eq!(payload["format"], "markdown");
    }

    #[test]
    fn attachment_mime_guess() {
        let ext = "photo.JPG".rsplit('.').next().unwrap().to_lowercase();
        assert_eq!(ext, "jpg");
        let mime = match ext.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" => format!("image/{ext}"),
            _ => "application/octet-stream".to_string(),
        };
        assert_eq!(mime, "image/jpg");
    }

    #[test]
    fn outbound_truncation_cap() {
        let long: String = "a".repeat(MAX_MESSAGE_LENGTH + 100);
        let body: String = long.chars().take(MAX_MESSAGE_LENGTH).collect();
        assert_eq!(body.len(), MAX_MESSAGE_LENGTH);
    }
}
