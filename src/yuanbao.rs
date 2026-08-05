//! Yuanbao platform adapter — port of hermes
//! `gateway/platforms/yuanbao.py` (functional core) @ v2026.8.3.
//!
//! Connects to the Yuanbao WebSocket gateway: sign-token HTTP auth
//! (HMAC-SHA256 over nonce+timestamp+app_key+app_secret), AUTH_BIND
//! handshake with connectId extraction, 30s ping/pong heartbeats with
//! a two-miss reconnect threshold, protobuf `InboundMessagePush`
//! decode → ulnclaw dispatch, and `send_c2c_message` /
//! `send_group_message` text replies with 4000-char chunking.
//!
//! Known differences: the inbound middleware pipeline collapses into a
//! direct decode→gate→dispatch path (recall guard, owner commands,
//! forwarded chat-history parsing, anchor patching not ported); media
//! resolution/upload and the sticker module ride private Yuanbao APIs
//! and are not ported (text-only replies; inbound media surfaces as
//! `[media: <type>]` notes); the slow-response notifier and reply
//! heartbeats are not ported.

use crate::messaging::{Dispatcher, MessageEvent};
use crate::pairing::PairingStore;
use crate::yuanbao_proto as proto;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_API_DOMAIN: &str = "https://bot.yuanbao.tencent.com";
const SIGN_TOKEN_PATH: &str = "/api/v5/robotLogic/sign-token";
const SIGN_RETRYABLE_CODE: i64 = 10099;
const SIGN_MAX_RETRIES: u32 = 3;
const SIGN_RETRY_DELAY_SECS: f64 = 1.0;
const CACHE_REFRESH_MARGIN_SECS: u64 = 60;

const CONNECT_TIMEOUT_SECS: u64 = 20;
const AUTH_TIMEOUT_SECS: u64 = 10;
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const PONG_TIMEOUT_SECS: u64 = 10;
const HEARTBEAT_TIMEOUT_THRESHOLD: u32 = 2;
const DEFAULT_SEND_TIMEOUT_SECS: u64 = 30;

/// hermes MAX_TEXT_CHUNK — Yuanbao single-message character limit.
const MAX_TEXT_CHUNK: usize = 4000;
/// hermes `_DEBOUNCE_WINDOW` for multi-part inbound aggregation.
const DEBOUNCE_WINDOW_SECS: f64 = 1.5;
/// hermes NO_RECONNECT_CLOSE_CODES.
const NO_RECONNECT_CLOSE_CODES: &[u16] = &[4012, 4013, 4014, 4018, 4019, 4021];

fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// `[messaging.yuanbao]` — Yuanbao WS-gateway adapter (hermes
/// `platforms.yuanbao` extra config + YUANBAO_* env vars).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct YuanbaoConfig {
    pub enabled: bool,
    /// App key (fallback `YUANBAO_APP_ID`).
    pub app_id: String,
    /// App secret (fallback `YUANBAO_APP_SECRET`).
    pub app_secret: String,
    /// Bot id (optional; the sign-token response returns it — fallback
    /// `YUANBAO_BOT_ID`).
    pub bot_id: String,
    /// WebSocket gateway URL (fallback `YUANBAO_WS_URL`).
    pub ws_url: String,
    /// API domain for sign-token (fallback `YUANBAO_API_DOMAIN`).
    pub api_domain: String,
    /// Optional route-env header (fallback `YUANBAO_ROUTE_ENV`).
    pub route_env: String,
    /// DM intake policy: pairing (default) | allowlist | open | disabled.
    pub dm_policy: String,
    pub allow_from: Vec<String>,
    /// Group intake policy: disabled (default) | allowlist | open
    /// (hermes group pairing resolves to closed).
    pub group_policy: String,
    pub group_allow_from: Vec<String>,
}

impl Default for YuanbaoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            app_id: String::new(),
            app_secret: String::new(),
            bot_id: String::new(),
            ws_url: String::new(),
            api_domain: String::new(),
            route_env: String::new(),
            dm_policy: "pairing".into(),
            allow_from: Vec::new(),
            group_policy: "disabled".into(),
            group_allow_from: Vec::new(),
        }
    }
}

fn env_or_none(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn resolve_app_id(cfg: &YuanbaoConfig) -> String {
    let trimmed = cfg.app_id.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    env_or_none("YUANBAO_APP_ID").unwrap_or_default()
}

pub fn resolve_app_secret(cfg: &YuanbaoConfig) -> String {
    let trimmed = cfg.app_secret.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    env_or_none("YUANBAO_APP_SECRET").unwrap_or_default()
}

pub fn resolve_bot_id(cfg: &YuanbaoConfig) -> String {
    let trimmed = cfg.bot_id.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    env_or_none("YUANBAO_BOT_ID").unwrap_or_default()
}

pub fn resolve_ws_url(cfg: &YuanbaoConfig) -> String {
    let trimmed = cfg.ws_url.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    env_or_none("YUANBAO_WS_URL").unwrap_or_default()
}

pub fn resolve_api_domain(cfg: &YuanbaoConfig) -> String {
    let trimmed = cfg.api_domain.trim();
    if !trimmed.is_empty() {
        return trimmed.trim_end_matches('/').to_string();
    }
    env_or_none("YUANBAO_API_DOMAIN")
        .map(|v| v.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_API_DOMAIN.to_string())
}

pub fn resolve_route_env(cfg: &YuanbaoConfig) -> String {
    let trimmed = cfg.route_env.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    env_or_none("YUANBAO_ROUTE_ENV").unwrap_or_default()
}

// ---------------------------------------------------------------------------
// SignManager (hermes `SignManager`)
// ---------------------------------------------------------------------------

/// hermes `compute_signature`: HMAC-SHA256(key=app_secret,
/// msg=nonce+timestamp+app_key+app_secret).
pub fn compute_signature(nonce: &str, timestamp: &str, app_key: &str, app_secret: &str) -> String {
    let plain = format!("{nonce}{timestamp}{app_key}{app_secret}");
    let mut mac = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()).expect("hmac key");
    mac.update(plain.as_bytes());
    let result = mac.finalize().into_bytes();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

/// hermes `build_timestamp`: Beijing-time ISO-8601 without milliseconds.
pub fn build_timestamp() -> String {
    let bj = chrono::FixedOffset::east_opt(8 * 3600).expect("+08:00 offset");
    chrono::Utc::now().with_timezone(&bj).format("%Y-%m-%dT%H:%M:%S+08:00").to_string()
}

#[derive(Debug, Clone)]
pub struct SignTokenEntry {
    token: String,
    bot_id: String,
    source: String,
    expire_ts: Instant,
}

pub struct SignManager {
    client: reqwest::Client,
    app_key: String,
    app_secret: String,
    api_domain: String,
    route_env: String,
    cache: Mutex<Option<SignTokenEntry>>,
}

impl SignManager {
    pub fn new(app_key: String, app_secret: String, api_domain: String, route_env: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            app_key,
            app_secret,
            api_domain,
            route_env,
            cache: Mutex::new(None),
        }
    }

    fn cache_valid(&self) -> Option<SignTokenEntry> {
        let cache = self.cache.lock().unwrap();
        cache.as_ref().and_then(|entry| {
            if entry.expire_ts > Instant::now() + Duration::from_secs(CACHE_REFRESH_MARGIN_SECS) {
                Some(entry.clone())
            } else {
                None
            }
        })
    }

    /// Cached token fetch (hermes `get_token`).
    pub async fn get_token(&self) -> std::result::Result<SignTokenEntry, String> {
        if let Some(entry) = self.cache_valid() {
            return Ok(entry);
        }
        let data = self.fetch().await?;
        let duration = data.get("duration").and_then(|v| v.as_u64()).unwrap_or(0);
        let ttl = if duration > 0 { duration } else { 3600 };
        let entry = SignTokenEntry {
            token: data.get("token").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            bot_id: data.get("bot_id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default(),
            source: data.get("source").and_then(|v| v.as_str()).unwrap_or("bot").to_string(),
            expire_ts: Instant::now() + Duration::from_secs(ttl),
        };
        if entry.token.is_empty() {
            return Err(format!("Sign token response missing token: {data}"));
        }
        *self.cache.lock().unwrap() = Some(entry.clone());
        Ok(entry)
    }

    /// hermes `force_refresh`.
    pub async fn force_refresh(&self) -> std::result::Result<SignTokenEntry, String> {
        self.cache.lock().unwrap().take();
        self.get_token().await
    }

    /// hermes `fetch` with retryable-code retries.
    async fn fetch(&self) -> std::result::Result<Value, String> {
        let url = format!("{}{}", self.api_domain, SIGN_TOKEN_PATH);
        let mut last_error = String::new();
        for attempt in 0..=SIGN_MAX_RETRIES {
            let mut nonce_bytes = [0u8; 16];
            getrandom_fill(&mut nonce_bytes);
            let nonce: String = nonce_bytes.iter().map(|b| format!("{b:02x}")).collect();
            let timestamp = build_timestamp();
            let signature = compute_signature(&nonce, &timestamp, &self.app_key, &self.app_secret);
            let payload = json!({
                "app_key": self.app_key,
                "nonce": nonce,
                "signature": signature,
                "timestamp": timestamp,
            });
            let mut request = self
                .client
                .post(&url)
                .timeout(Duration::from_secs(10))
                .header("Content-Type", "application/json")
                .header("X-AppVersion", app_version())
                .header("X-OperationSystem", std::env::consts::OS)
                .header("X-Instance-Id", proto::INSTANCE_ID.to_string())
                .header("X-Bot-Version", app_version());
            if !self.route_env.is_empty() {
                request = request.header("X-Route-Env", &self.route_env);
            }
            let response = request
                .json(&payload)
                .send()
                .await
                .map_err(|e| format!("sign-token request: {e}"))?;
            let status = response.status();
            let raw = response.text().await.unwrap_or_default();
            if status.as_u16() != 200 {
                return Err(format!("Sign token API returned {}: {}", status.as_u16(), truncate_str(&raw, 200)));
            }
            let result: Value = serde_json::from_str(&raw).map_err(|e| format!("Sign token response parse error: {e}"))?;
            let code = result.get("code").and_then(|v| v.as_i64());
            if code == Some(0) {
                let data = result.get("data").filter(|v| v.is_object()).cloned().ok_or_else(|| {
                    format!("Sign token response missing 'data' field: {result}")
                })?;
                return Ok(data);
            }
            if code == Some(SIGN_RETRYABLE_CODE) && attempt < SIGN_MAX_RETRIES {
                last_error = format!("sign-token retryable code={SIGN_RETRYABLE_CODE}");
                tokio::time::sleep(Duration::from_secs_f64(SIGN_RETRY_DELAY_SECS)).await;
                continue;
            }
            let msg = result.get("msg").and_then(|v| v.as_str()).unwrap_or("");
            return Err(format!("Sign token error: code={code:?}, msg={msg}"));
        }
        Err(format!("Sign token failed: max retries exceeded ({last_error})"))
    }
}

fn getrandom_fill(bytes: &mut [u8]) {
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        if file.read_exact(bytes).is_ok() {
            return;
        }
    }
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0x9E3779B97F4A7C15);
    for slot in bytes.iter_mut() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *slot = (seed >> 33) as u8;
    }
}

fn truncate_str(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

// ---------------------------------------------------------------------------
// Adapter state
// ---------------------------------------------------------------------------

struct Runner {
    cfg: YuanbaoConfig,
    sign: Arc<SignManager>,
    dispatcher: Arc<Dispatcher>,
    pairing: Option<Arc<PairingStore>>,
    bot_id: Mutex<String>,
    dedup: Mutex<HashMap<String, Instant>>,
    connected: AtomicBool,
    /// Outstanding ping without pong (hermes heartbeat miss tracking).
    pong_pending: AtomicBool,
    /// Debounce buffer: sender key -> (raw frames, generation).
    inbound_buffer: Mutex<HashMap<String, (Vec<Vec<u8>>, u64)>>,
}

impl Runner {
    fn is_duplicate(&self, key: &str) -> bool {
        let mut seen = self.dedup.lock().unwrap();
        dedup_check(&mut seen, key)
    }
}

fn dedup_check(seen: &mut HashMap<String, Instant>, key: &str) -> bool {
    if seen.len() > 1024 {
        seen.retain(|_, at| at.elapsed() < Duration::from_secs(300));
    }
    if let Some(at) = seen.get(key) {
        if at.elapsed() < Duration::from_secs(300) {
            return true;
        }
    }
    seen.insert(key.to_string(), Instant::now());
    false
}

/// Start the Yuanbao adapter (hermes `YuanbaoAdapter.connect` +
/// ConnectionManager lifecycle).
pub async fn run(cfg: YuanbaoConfig, dispatcher: Arc<Dispatcher>, pairing: Option<Arc<PairingStore>>) {
    let app_id = resolve_app_id(&cfg);
    let app_secret = resolve_app_secret(&cfg);
    if app_id.is_empty() || app_secret.is_empty() {
        eprintln!("[yuanbao] disabled: YUANBAO_APP_ID and YUANBAO_APP_SECRET are required");
        return;
    }
    let ws_url = resolve_ws_url(&cfg);
    if ws_url.is_empty() {
        eprintln!("[yuanbao] disabled: no ws_url configured (set messaging.yuanbao.ws_url or YUANBAO_WS_URL)");
        return;
    }
    let sign = Arc::new(SignManager::new(
        app_id.clone(),
        app_secret,
        resolve_api_domain(&cfg),
        resolve_route_env(&cfg),
    ));
    let runner = Arc::new(Runner {
        cfg: cfg.clone(),
        sign,
        dispatcher,
        pairing,
        bot_id: Mutex::new(resolve_bot_id(&cfg)),
        dedup: Mutex::new(HashMap::new()),
        connected: AtomicBool::new(false),
        pong_pending: AtomicBool::new(false),
        inbound_buffer: Mutex::new(HashMap::new()),
    });

    register_sender(runner.clone(), ws_url.clone());

    let mut backoff_idx: usize = 0;
    loop {
        match run_session(&runner, &ws_url).await {
            Ok(()) => backoff_idx = 0,
            Err(code) => {
                if NO_RECONNECT_CLOSE_CODES.contains(&code) {
                    eprintln!("[yuanbao] close code {code} is non-recoverable, NOT reconnecting");
                    return;
                }
                let delay = [2u64, 5, 10, 30, 60][backoff_idx.min(4)];
                eprintln!("[yuanbao] session ended (code={code}) — reconnecting in {delay}s");
                tokio::time::sleep(Duration::from_secs(delay)).await;
                backoff_idx = (backoff_idx + 1).min(4);
            }
        }
    }
}

/// One WS session: sign-token → connect → AUTH_BIND → heartbeat + recv.
/// Returns Err(close_code) on failure/close.
async fn run_session(runner: &Arc<Runner>, ws_url: &str) -> std::result::Result<(), u16> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    // Step 1: sign token.
    let token_entry = match runner.sign.get_token().await {
        Ok(entry) => entry,
        Err(e) => {
            eprintln!("[yuanbao] sign-token failed: {e}");
            return Err(0);
        }
    };
    if !token_entry.bot_id.is_empty() {
        *runner.bot_id.lock().unwrap() = token_entry.bot_id.clone();
    }

    // Step 2: WS connect.
    let connect = tokio::time::timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS + 10), tokio_tungstenite::connect_async(ws_url)).await;
    let (ws, _) = match connect {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            eprintln!("[yuanbao] WS connect failed: {e}");
            return Err(0);
        }
        Err(_) => {
            eprintln!("[yuanbao] WS connect timed out");
            return Err(0);
        }
    };
    let (mut sink, mut stream) = ws.split();
    eprintln!("[yuanbao] WebSocket connected to {ws_url}");

    // Step 3: AUTH_BIND → BIND_ACK.
    let auth_msg_id = new_uuid();
    let auth_bytes = proto::encode_auth_bind(
        "ybBot",
        &runner.bot_id.lock().unwrap().clone(),
        if token_entry.source.is_empty() { "bot" } else { &token_entry.source },
        &token_entry.token,
        &auth_msg_id,
        &app_version(),
        std::env::consts::OS,
        &app_version(),
        &resolve_route_env(&runner.cfg),
    );
    if sink.send(WsMessage::Binary(auth_bytes)).await.is_err() {
        return Err(0);
    }

    let deadline = Instant::now() + Duration::from_secs(AUTH_TIMEOUT_SECS);
    let connect_id = loop {
        if Instant::now() >= deadline {
            eprintln!("[yuanbao] AUTH_BIND timeout waiting for BIND_ACK");
            return Err(0);
        }
        let next = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), stream.next()).await;
        let Ok(Some(Ok(message))) = next else { return Err(0) };
        let WsMessage::Binary(raw) = message else { continue };
        let msg = proto::decode_conn_msg(&raw);
        if msg.head.cmd_type == proto::CMD_TYPE_RESPONSE && msg.head.cmd == proto::CMD_AUTH_BIND {
            match proto::decode_auth_bind_rsp(&msg.data) {
                Ok(connect_id) => break connect_id,
                Err(e) => {
                    eprintln!("[yuanbao] BIND_ACK error: {e}");
                    return Err(0);
                }
            }
        }
    };
    runner.connected.store(true, Ordering::SeqCst);
    eprintln!("[yuanbao] connected connectId={connect_id} botId={}", runner.bot_id.lock().unwrap());

    // Step 4: heartbeat + receive loops.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let (hb_tx, mut hb_rx) = tokio::sync::mpsc::channel::<()>(4);
    let runner_hb = runner.clone();
    let heartbeat = tokio::spawn(async move {
        let mut missed: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
            let msg_id = new_uuid();
            runner_hb.pong_pending.store(true, Ordering::SeqCst);
            if hb_tx.send(()).await.is_err() {
                return;
            }
            let _ = msg_id;
            tokio::time::sleep(Duration::from_secs(PONG_TIMEOUT_SECS)).await;
            if runner_hb.pong_pending.load(Ordering::SeqCst) {
                missed += 1;
                eprintln!("[yuanbao] PONG timeout ({missed}/{HEARTBEAT_TIMEOUT_THRESHOLD})");
                if missed >= HEARTBEAT_TIMEOUT_THRESHOLD {
                    eprintln!("[yuanbao] heartbeat threshold exceeded, triggering reconnect");
                    runner_hb.connected.store(false, Ordering::SeqCst);
                    return;
                }
            } else {
                missed = 0;
            }
        }
    });

    let result: std::result::Result<(), u16> = loop {
        tokio::select! {
            _ = hb_rx.recv() => {
                let msg_id = new_uuid();
                if sink.send(WsMessage::Binary(proto::encode_ping(&msg_id))).await.is_err() {
                    break Err(0);
                }
            }
            outbound = out_rx.recv() => {
                let Some(bytes) = outbound else { break Ok(()) };
                if sink.send(WsMessage::Binary(bytes)).await.is_err() {
                    break Err(0);
                }
            }
            incoming = stream.next() => {
                let Some(message) = incoming else { break Err(0) };
                let message = match message {
                    Ok(message) => message,
                    Err(e) => break Err(close_code_from_error(&e.to_string())),
                };
                let WsMessage::Binary(raw) = message else { continue };
                match handle_frame(runner, &raw, &out_tx).await {
                    FrameAction::Continue => {}
                    FrameAction::Close(code) => break Err(code),
                }
            }
        }
    };
    runner.connected.store(false, Ordering::SeqCst);
    heartbeat.abort();
    result
}

enum FrameAction {
    Continue,
    Close(u16),
}

/// hermes `_handle_frame` (core paths).
async fn handle_frame(
    runner: &Arc<Runner>,
    raw: &[u8],
    out_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
) -> FrameAction {
    let msg = proto::decode_conn_msg(raw);
    let head = &msg.head;
    match head.cmd_type {
        proto::CMD_TYPE_RESPONSE if head.cmd == proto::CMD_PING => {
            runner.pong_pending.store(false, Ordering::SeqCst);
            FrameAction::Continue
        }
        proto::CMD_TYPE_RESPONSE if head.cmd == "send_group_heartbeat" || head.cmd == "send_private_heartbeat" => {
            FrameAction::Continue
        }
        proto::CMD_TYPE_RESPONSE => {
            // Response to an outbound RPC (send_c2c/group_message): the
            // send path correlates by msg_id.
            pending_responses().lock().unwrap().insert(head.msg_id.clone(), msg.data.clone());
            FrameAction::Continue
        }
        proto::CMD_TYPE_PUSH => {
            // Kickout is fatal — another login replaced this session.
            if head.cmd == "kickout" {
                eprintln!("[yuanbao] kickout received — session replaced, not reconnecting");
                return FrameAction::Close(4013);
            }
            if head.need_ack {
                let ack = proto::encode_push_ack(head);
                out_tx.send(ack).await.ok();
            }
            if !msg.data.is_empty() {
                buffer_inbound(runner, out_tx, msg.data.clone());
            }
            FrameAction::Continue
        }
        _ => FrameAction::Continue,
    }
}

fn close_code_from_error(text: &str) -> u16 {
    for code in NO_RECONNECT_CLOSE_CODES {
        if text.contains(&code.to_string()) {
            return *code;
        }
    }
    0
}

/// Outbound RPC response correlation (msg_id -> response payload).
static PENDING_RESPONSES: std::sync::OnceLock<Mutex<HashMap<String, Vec<u8>>>> = std::sync::OnceLock::new();

fn pending_responses() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    PENDING_RESPONSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn new_uuid() -> String {
    let mut bytes = [0u8; 16];
    getrandom_fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// ---------------------------------------------------------------------------
// Inbound (hermes pipeline essence: decode → gate → dispatch)
// ---------------------------------------------------------------------------

/// hermes `_push_to_inbound` debounce: aggregate frames per sender key
/// within a short window, then run one merged pipeline pass.
fn buffer_inbound(runner: &Arc<Runner>, out_tx: &tokio::sync::mpsc::Sender<Vec<u8>>, data: Vec<u8>) {
    // Lightweight sender-key extraction (hermes `_extract_sender_key`).
    let key = proto::decode_inbound_push(&data)
        .map(|push| format!("{}:{}", push.from_account, push.group_code))
        .unwrap_or_else(|| format!("__unknown_{}", data.len()));
    let generation;
    {
        let mut buffer = runner.inbound_buffer.lock().unwrap();
        let entry = buffer.entry(key.clone()).or_insert_with(|| (Vec::new(), 0));
        entry.0.push(data);
        entry.1 += 1;
        generation = entry.1;
    }
    let runner = runner.clone();
    let out_tx = out_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs_f64(DEBOUNCE_WINDOW_SECS)).await;
        let frames = {
            let mut buffer = runner.inbound_buffer.lock().unwrap();
            match buffer.get(&key) {
                Some((frames, gen)) if *gen == generation => buffer.remove(&key).map(|(frames, _)| frames),
                _ => None,
            }
        };
        if let Some(frames) = frames {
            handle_inbound_frames(&runner, &frames, &out_tx).await;
        }
    });
}

/// Decode + gate + dispatch one debounced batch of push frames (hermes
/// pipeline essence).
async fn handle_inbound_frames(runner: &Arc<Runner>, frames: &[Vec<u8>], out_tx: &tokio::sync::mpsc::Sender<Vec<u8>>) {
    let mut merged: Option<proto::InboundPush> = None;
    for data in frames {
        let Some(push) = proto::decode_inbound_push(data) else { continue };
        match merged.as_mut() {
            Some(into) => {
                into.msg_body.extend(push.msg_body);
                if into.msg_id.is_empty() {
                    into.msg_id = push.msg_id;
                }
                if into.msg_key.is_empty() {
                    into.msg_key = push.msg_key;
                }
            }
            None => merged = Some(push),
        }
    }
    let Some(push) = merged else { return };
    if push.from_account.is_empty() {
        return;
    }
    let dedup_key = if push.msg_key.is_empty() { push.msg_id.clone() } else { push.msg_key.clone() };
    if !dedup_key.is_empty() && runner.is_duplicate(&dedup_key) {
        return;
    }

    let is_group = !push.group_code.is_empty();
    let (chat_id, gated) = if is_group {
        let allowed = match runner.cfg.group_policy.as_str() {
            "allowlist" => runner.cfg.group_allow_from.iter().any(|g| g.trim() == push.group_code),
            "open" => open_opted_in(),
            _ => false, // disabled + pairing (hermes: group pairing closed)
        };
        (push.group_code.clone(), allowed)
    } else {
        let intake = match runner.cfg.dm_policy.as_str() {
            "disabled" => false,
            "allowlist" => runner.cfg.allow_from.iter().any(|a| a.trim() == push.from_account),
            "pairing" => true,
            "open" => open_opted_in(),
            _ => false,
        };
        let authorized = runner.cfg.allow_from.iter().any(|a| a.trim() == push.from_account)
            || runner.pairing.as_ref().map(|s| s.is_approved("yuanbao", &push.from_account)).unwrap_or(false)
            || runner.cfg.dm_policy == "open";
        (push.from_account.clone(), intake && authorized)
    };

    if !gated {
        eprintln!(
            "[yuanbao] refusing message from {} — add it to messaging.yuanbao.allow_from/group_allow_from or approve a pairing code",
            push.from_account
        );
        if !is_group {
            if let Some(store) = &runner.pairing {
                if let Some(reply) = crate::messaging::pairing_offer_public(store, "yuanbao", &push.from_account, &push.sender_nickname) {
                    send_text_via(runner, out_tx, &chat_id, "", &reply).await;
                }
            }
        }
        return;
    }

    // Extract text + note media elements (media resolution rides private
    // Yuanbao APIs and is not ported).
    let mut text_parts: Vec<String> = Vec::new();
    for element in &push.msg_body {
        match element.msg_type.as_str() {
            "TIMTextElem" => {
                if !element.msg_content.text.is_empty() {
                    text_parts.push(element.msg_content.text.clone());
                }
            }
            other if !other.is_empty() => {
                text_parts.push(format!("[media: {other}]"));
            }
            _ => {}
        }
    }
    let text = text_parts.join("\n");
    if text.trim().is_empty() {
        return;
    }

    eprintln!(
        "[yuanbao] inbound from={} {} text_len={}",
        truncate_str(&push.from_account, 12),
        if is_group { format!("group={}", push.group_code) } else { "dm".into() },
        text.chars().count()
    );

    let sender_name = if push.sender_nickname.is_empty() {
        push.from_account.clone()
    } else {
        push.sender_nickname.clone()
    };
    let mut event = MessageEvent {
        platform: "yuanbao".into(),
        chat_id: chat_id.clone(),
        sender_id: push.from_account.clone(),
        sender_name,
        text,
        message_id: push.msg_id.clone(),
        attachments: Vec::new(),
    };
    if !crate::messaging::pre_gateway_dispatch_gate_public(&mut event).await {
        return;
    }
    let outcome = match runner.dispatcher.handle_event(event).await {
        Ok(outcome) => outcome,
        Err(e) => crate::messaging::DispatchOutcome {
            reply: format!("error: {e}"),
            transcript_echoes: Vec::new(),
        },
    };
    for echo in &outcome.transcript_echoes {
        send_text_via(runner, out_tx, &chat_id, &push.group_code, echo).await;
    }
    let (reply_text, _media_paths) = crate::messaging::extract_media_tags(&outcome.reply);
    if !reply_text.trim().is_empty() {
        send_text_via(runner, out_tx, &chat_id, &push.group_code, &reply_text).await;
    }
}

fn open_opted_in() -> bool {
    for var in ["GATEWAY_ALLOW_ALL_USERS", "YUANBAO_ALLOW_ALL_USERS"] {
        if matches!(
            std::env::var(var).unwrap_or_default().to_lowercase().as_str(),
            "true" | "1" | "yes"
        ) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Outbound text (hermes MessageSender.send_text essence)
// ---------------------------------------------------------------------------

/// Paragraph-aware chunking at MAX_TEXT_CHUNK (hermes truncate_message +
/// MarkdownProcessor essentials).
pub fn chunk_markdown_text(content: &str, max: usize) -> Vec<String> {
    if content.chars().count() <= max {
        return vec![content.to_string()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for paragraph in content.split("\n\n") {
        let paragraph = paragraph.trim_matches('\n');
        if paragraph.is_empty() {
            continue;
        }
        if paragraph.chars().count() > max {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let mut piece = String::new();
            let mut len = 0usize;
            for ch in paragraph.chars() {
                if len + 1 > max {
                    chunks.push(std::mem::take(&mut piece));
                    len = 0;
                }
                piece.push(ch);
                len += 1;
            }
            if !piece.is_empty() {
                chunks.push(piece);
            }
            continue;
        }
        let candidate = if current.is_empty() {
            paragraph.to_string()
        } else {
            format!("{current}\n\n{paragraph}")
        };
        if candidate.chars().count() <= max {
            current = candidate;
        } else {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current = paragraph.to_string();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Encode + send one text chunk over the WS, wait for the RPC response.
async fn send_text_chunk_ws(
    runner: &Arc<Runner>,
    out_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    chat_id: &str,
    group_code: &str,
    chunk: &str,
) -> std::result::Result<(), String> {
    let msg_id = new_uuid();
    let from_account = runner.bot_id.lock().unwrap().clone();
    let element = proto::MsgBodyElement {
        msg_type: "TIMTextElem".into(),
        msg_content: proto::MsgContent {
            text: chunk.to_string(),
            ..Default::default()
        },
    };
    let bytes = if group_code.is_empty() {
        proto::encode_send_c2c_message(chat_id, &[element], &from_account, &msg_id, 0, None, "", "")
    } else {
        proto::encode_send_group_message(group_code, &[element], &from_account, &msg_id, "", "", None, "", "")
    };
    out_tx.send(bytes).await.map_err(|e| format!("send channel closed: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(DEFAULT_SEND_TIMEOUT_SECS);
    loop {
        if Instant::now() >= deadline {
            return Err(format!("yuanbao send timeout waiting for response (req_id={msg_id})"));
        }
        if let Some(data) = pending_responses().lock().unwrap().remove(&msg_id) {
            // SendC2CMessageRsp/SendGroupMessageRsp: field 1 = code.
            let code = proto::decode_send_rsp_code(&data);
            if code != 0 {
                return Err(format!("yuanbao send response code={code} (req_id={msg_id})"));
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Send text with chunking + retries (hermes send_text semantics).
async fn send_text_via(runner: &Arc<Runner>, out_tx: &tokio::sync::mpsc::Sender<Vec<u8>>, chat_id: &str, group_code: &str, content: &str) {
    let chunks = chunk_markdown_text(content, MAX_TEXT_CHUNK);
    for chunk in chunks {
        let mut last_error = String::new();
        for attempt in 0..3u32 {
            match send_text_chunk_ws(runner, out_tx, chat_id, group_code, &chunk).await {
                Ok(()) => {
                    last_error = String::new();
                    break;
                }
                Err(e) => {
                    last_error = e;
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    }
                }
            }
        }
        if !last_error.is_empty() {
            eprintln!("[yuanbao] send failed to {chat_id}: {last_error}");
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// PlatformSender (clarify gateway / webhook delivery)
// ---------------------------------------------------------------------------

struct YuanbaoSender {
    runner: Arc<Runner>,
    ws_url: String,
}

#[async_trait::async_trait]
impl crate::messaging::PlatformSender for YuanbaoSender {
    async fn send_text(&self, chat_id: &str, text: &str) {
        // Out-of-session sends open a short-lived WS with the same auth
        // flow (hermes keeps one connection; ulnclaw reuses the live one
        // through the clarify task-local when available, else connects).
        send_direct(&self.runner, &self.ws_url, chat_id, text).await;
    }
}

fn register_sender(runner: Arc<Runner>, ws_url: String) {
    crate::messaging::register_platform_sender("yuanbao", Arc::new(YuanbaoSender { runner, ws_url }));
}

/// One-shot direct send (own ephemeral session).
async fn send_direct(runner: &Arc<Runner>, ws_url: &str, chat_id: &str, text: &str) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let Ok(token_entry) = runner.sign.get_token().await else {
        eprintln!("[yuanbao] direct send: sign-token failed");
        return;
    };
    if !token_entry.bot_id.is_empty() {
        *runner.bot_id.lock().unwrap() = token_entry.bot_id.clone();
    }
    let Ok(Ok((ws, _))) = tokio::time::timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS + 10), tokio_tungstenite::connect_async(ws_url)).await
    else {
        eprintln!("[yuanbao] direct send: WS connect failed");
        return;
    };
    let (mut sink, mut stream) = ws.split();
    let auth_msg_id = new_uuid();
    let auth_bytes = proto::encode_auth_bind(
        "ybBot",
        &runner.bot_id.lock().unwrap().clone(),
        if token_entry.source.is_empty() { "bot" } else { &token_entry.source },
        &token_entry.token,
        &auth_msg_id,
        &app_version(),
        std::env::consts::OS,
        &app_version(),
        &resolve_route_env(&runner.cfg),
    );
    if sink.send(WsMessage::Binary(auth_bytes)).await.is_err() {
        return;
    }
    // Wait for BIND_ACK.
    let deadline = Instant::now() + Duration::from_secs(AUTH_TIMEOUT_SECS);
    loop {
        if Instant::now() >= deadline {
            return;
        }
        let Ok(Some(Ok(message))) = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), stream.next()).await
        else {
            return;
        };
        let WsMessage::Binary(raw) = message else { continue };
        let msg = proto::decode_conn_msg(&raw);
        if msg.head.cmd_type == proto::CMD_TYPE_RESPONSE && msg.head.cmd == proto::CMD_AUTH_BIND {
            if proto::decode_auth_bind_rsp(&msg.data).is_err() {
                return;
            }
            break;
        }
    }
    let msg_id = new_uuid();
    let from_account = runner.bot_id.lock().unwrap().clone();
    let element = proto::MsgBodyElement {
        msg_type: "TIMTextElem".into(),
        msg_content: proto::MsgContent {
            text: text.to_string(),
            ..Default::default()
        },
    };
    let bytes = proto::encode_send_c2c_message(chat_id, &[element], &from_account, &msg_id, 0, None, "", "");
    sink.send(WsMessage::Binary(bytes)).await.ok();
    // Give the server a moment to ack before closing.
    tokio::time::sleep(Duration::from_secs(1)).await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_hmac_sha256_vector() {
        // HMAC-SHA256(key=app_secret, msg=nonce+timestamp+app_key+app_secret)
        let signature = compute_signature(
            "abc",
            "2026-08-06T10:00:00+08:00",
            "appkey",
            "secret123",
        );
        assert_eq!(
            signature,
            "0806db561aa469139a2a2c249268026bbf9ea4304331590db060496a2ddc9999"
        );
    }

    #[test]
    fn timestamp_is_beijing_iso8601() {
        let ts = build_timestamp();
        assert!(
            regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+08:00$")
                .unwrap()
                .is_match(&ts),
            "unexpected timestamp: {ts}"
        );
    }

    #[test]
    fn chunk_markdown_respects_limit() {
        assert_eq!(chunk_markdown_text("short", MAX_TEXT_CHUNK), vec!["short"]);

        let mut long = String::new();
        for i in 0..100 {
            long.push_str(&format!("Paragraph {i} with body text.\n\n"));
        }
        let chunks = chunk_markdown_text(&long, 400);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 400);
        }

        let giant = "x".repeat(MAX_TEXT_CHUNK * 2 + 10);
        let chunks = chunk_markdown_text(&giant, MAX_TEXT_CHUNK);
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn config_resolution_prefers_explicit_values() {
        let mut cfg = YuanbaoConfig::default();
        cfg.app_id = "cfg-app".into();
        cfg.api_domain = "https://example.com/".into();
        assert_eq!(resolve_app_id(&cfg), "cfg-app");
        assert_eq!(resolve_api_domain(&cfg), "https://example.com");
        // Defaults fall through.
        let empty = YuanbaoConfig::default();
        assert_eq!(resolve_api_domain(&empty), DEFAULT_API_DOMAIN);
        assert!(resolve_ws_url(&empty).is_empty());
    }

    #[test]
    fn uuid_shape() {
        let id = new_uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
        assert_ne!(new_uuid(), id);
    }

    #[test]
    fn dedup_flags_repeats() {
        let mut seen: HashMap<String, Instant> = HashMap::new();
        assert!(!dedup_check(&mut seen, "k1"));
        assert!(dedup_check(&mut seen, "k1"));
        assert!(!dedup_check(&mut seen, "k2"));
        // Expired entries pass again.
        seen.insert("k3".into(), Instant::now() - Duration::from_secs(301));
        assert!(!dedup_check(&mut seen, "k3"));
    }
}
