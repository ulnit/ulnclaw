//! HTTP gateway — OpenAI-compatible API server.
//!
//! Minimal Rust port of hermes' `gateway/platforms/api_server.py`:
//!   - `GET  /health`, `GET /health/detailed` — probes (always unauthenticated)
//!   - `GET  /v1/models`, `GET /api/model/options`, `GET /v1/capabilities`
//!   - `POST /v1/chat/completions` — OpenAI Chat Completions format; opt-in
//!     session continuity via the `X-Ulnclaw-Session-Id` header;
//!     `stream: true` returns SSE `chat.completion.chunk` events
//!   - `POST /api/sessions/{id}/chat/stream` — SSE variant of session chat
//!   - `GET/POST /api/sessions`, `GET/DELETE /api/sessions/{id}`,
//!     `GET /api/sessions/:id/messages`, `POST /api/sessions/:id/chat`
//!   - `PATCH /api/sessions/:id` — update title / end_reason
//!   - `POST /api/sessions/:id/fork` — branch a session into a child
//!   - `POST /api/sessions/:id/model` — lock a session to a model
//!   - `GET /api/pets/config`, `GET /api/pets`,
//!     `GET /api/pets/:slug/spritesheet` — petdex mascot surfaces
//!   - `POST /api/pets/hatch`, `GET /api/pets/hatch/:id`,
//!     `POST /api/pets/hatch/:id/pick|cancel`,
//!     `GET /api/pets/hatch/:id/draft/:index` — hatch job pipeline
//!     for the desktop hatch overlay
//!   - `GET/POST /api/projects` (+ `/active`, `/repos`, `/scan`,
//!     `/:id`, `/:id/folders`, `/:id/primary`, `/:id/archive|restore`),
//!   - `GET/POST /api/kanban/boards`, `POST /api/kanban/boards/:slug/switch`,
//!     `GET/POST /api/kanban/tasks`, `GET /api/kanban/tasks/:id`,
//!     `POST /api/kanban/tasks/:id/complete|block|unblock|comment|link|claim`
//!     — kanban board API shared with the CLI + agent tools
//!   - `GET  /api/sessions/:id/recap` — instant local activity recap
//!   - `POST /v1/runs`, `GET /v1/runs`, `GET /v1/runs/{id}`,
//!     `POST /v1/runs/:id/stop`
//!   - `GET/POST /api/jobs`, `GET/PATCH/DELETE /api/jobs/{id}`,
//!     `POST /api/jobs/{id}/pause|resume|run`,
//!     `GET /api/jobs/delivery-targets`, `POST /api/jobs/fire` —
//!     cron job management + external delivery + Chronos fire webhook
//!   - `GET /v1/skills`, `GET /v1/toolsets` — discovery endpoints
//!
//! Bearer-token auth via `[gateway] key` / `ULNCLAW_GATEWAY_KEY` (optional;
//! when unset the gateway is open — bind it to localhost).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

mod kanban;
mod pets;
mod projects;

use crate::agent::Agent;
use crate::cron::{CronJob, CronStore};
use crate::error::{AgentError, Result};
use crate::provider::{Message, Role};
use crate::session::sqlite::SqliteSessionStore;
use crate::session::SessionStore;

/// Header carrying the session id (request + response).
pub const SESSION_HEADER: &str = "x-ulnclaw-session-id";

tokio::task_local! {
    static RUN_ID: String;
}

/// The run id of the current task (set for agent runs started via
/// `/v1/runs`); `None` in chat-completions and other contexts.
pub fn current_run_id() -> Option<String> {
    RUN_ID.try_with(|r| r.clone()).ok()
}

/// A dangerous-command approval waiting for `POST /v1/runs/:id/approval`.
pub struct PendingApproval {
    pub command: String,
    pub reason: String,
    pub respond: tokio::sync::oneshot::Sender<String>,
}

/// How a pending approval ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Approved,
    Denied,
    /// No decision arrived within the timeout window — fail closed.
    TimedOut,
}

/// Routes approval requests raised inside agent runs to the HTTP approval
/// flow (and remembers `always`/`session` decisions).
pub struct ApprovalRouter {
    channels: std::sync::Mutex<HashMap<String, (String, tokio::sync::mpsc::UnboundedSender<PendingApproval>)>>,
    allow_always: tokio::sync::Mutex<std::collections::HashSet<String>>,
    allow_session: tokio::sync::Mutex<HashMap<String, std::collections::HashSet<String>>>,
    /// Fail-closed wait limit for a human decision (hermes default 300s).
    timeout: std::time::Duration,
    /// Where `always` grants are persisted across restarts (optional).
    persist_path: Option<std::path::PathBuf>,
}

impl ApprovalRouter {
    pub fn new() -> Arc<Self> {
        Self::with_options(std::time::Duration::from_secs(300), None)
    }

    /// Build a router with a custom approval timeout and an optional
    /// persistence file for `always` grants (hermes keeps permanent
    /// approvals on disk so they survive restarts).
    pub fn with_options(timeout: std::time::Duration, persist_path: Option<std::path::PathBuf>) -> Arc<Self> {
        let mut allow_always = std::collections::HashSet::new();
        if let Some(path) = &persist_path {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(value) = serde_json::from_str::<Value>(&content) {
                    if let Some(items) = value.get("always").and_then(|v| v.as_array()) {
                        for item in items {
                            if let Some(command) = item.as_str() {
                                allow_always.insert(command.to_string());
                            }
                        }
                    }
                }
            }
        }
        Arc::new(Self {
            channels: std::sync::Mutex::new(HashMap::new()),
            allow_always: tokio::sync::Mutex::new(allow_always),
            allow_session: tokio::sync::Mutex::new(HashMap::new()),
            timeout,
            persist_path,
        })
    }

    /// Wire a run's approval channel (called when the run starts).
    pub fn register(
        &self,
        run_id: &str,
        session_id: &str,
        sender: tokio::sync::mpsc::UnboundedSender<PendingApproval>,
    ) {
        self.channels
            .lock()
            .unwrap()
            .insert(run_id.to_string(), (session_id.to_string(), sender));
    }

    pub fn unregister(&self, run_id: &str) {
        self.channels.lock().unwrap().remove(run_id);
    }

    /// Raised by the agent's approve callback inside a run task. Blocks until
    /// the HTTP client resolves the approval (or the channel disappears).
    pub async fn request(&self, run_id: &str, reason: String, command: String) -> bool {
        matches!(
            self.request_outcome(run_id, reason, command).await,
            ApprovalOutcome::Approved
        )
    }

    /// Like `request`, but distinguishes an explicit deny from a timeout
    /// (hermes: a timeout is fail-closed "no response", not a user deny).
    pub async fn request_outcome(
        &self,
        run_id: &str,
        reason: String,
        command: String,
    ) -> ApprovalOutcome {
        if self.allow_always.lock().await.contains(&command) {
            return ApprovalOutcome::Approved;
        }
        let (session_id, send_result) = {
            let channels = self.channels.lock().unwrap();
            match channels.get(run_id) {
                Some((session_id, sender)) => {
                    let (respond, response) = tokio::sync::oneshot::channel();
                    let sent = sender
                        .send(PendingApproval {
                            command: command.clone(),
                            reason,
                            respond,
                        })
                        .is_ok();
                    (Some(session_id.clone()), sent.then_some(response))
                }
                None => (None, None),
            }
        };
        if let Some(session_id) = &session_id {
            if self
                .allow_session
                .lock()
                .await
                .get(session_id)
                .map(|allowed| allowed.contains(&command))
                .unwrap_or(false)
            {
                return ApprovalOutcome::Approved;
            }
        }
        let Some(response) = send_result else {
            return ApprovalOutcome::Denied;
        };
        let decision = match tokio::time::timeout(self.timeout, response).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => return ApprovalOutcome::Denied,
            Err(_) => return ApprovalOutcome::TimedOut,
        };
        match decision.as_str() {
            "once" => ApprovalOutcome::Approved,
            "always" => {
                self.grant_always(command).await;
                ApprovalOutcome::Approved
            }
            "session" => {
                if let Some(session_id) = session_id {
                    self.allow_session
                        .lock()
                        .await
                        .entry(session_id)
                        .or_default()
                        .insert(command);
                }
                ApprovalOutcome::Approved
            }
            _ => ApprovalOutcome::Denied,
        }
    }

    /// Remember a command for the gateway's lifetime and persist it across
    /// restarts (when a persist path is configured).
    pub async fn grant_always(&self, command: String) {
        self.allow_always.lock().await.insert(command.clone());
        let Some(path) = &self.persist_path else { return };
        let commands: Vec<Value> = self
            .allow_always
            .lock()
            .await
            .iter()
            .map(|c| Value::String(c.clone()))
            .collect();
        let payload = json!({"always": commands});
        std::fs::write(path, serde_json::to_string_pretty(&payload).unwrap_or_default()).ok();
    }
}

/// Build the gateway approve callback: routes confirm-tier commands raised
/// inside a run to the HTTP approval flow.  On timeout the run's pending
/// approval is cleaned up (fail closed) so the run never parks forever.
///
/// `state` is late-bound (the agent owning the callback is constructed
/// before the GatewayState that wraps it).
pub fn gateway_approve_fn(
    router: Arc<ApprovalRouter>,
    state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
) -> crate::tools::context::ApproveFn {
    Arc::new(move |reason, command| {
        let router = router.clone();
        let state = state.clone();
        Box::pin(async move {
            let Some(run_id) = current_run_id() else {
                // No run context (e.g. chat-completions): deny by design.
                return false;
            };
            match router.request_outcome(&run_id, reason, command).await {
                ApprovalOutcome::Approved => true,
                ApprovalOutcome::Denied => false,
                ApprovalOutcome::TimedOut => {
                    if let Some(state) = state.get() {
                        state.pending_approvals.lock().await.remove(&run_id);
                        let mut runs = state.runs.lock().await;
                        if let Some(run) = runs.get_mut(&run_id) {
                            if run.status == "waiting_for_approval" {
                                run.status = "running".to_string();
                            }
                            if let Some(approval) = run.approval.as_mut() {
                                approval["resolved"] = json!("timeout");
                            }
                        }
                    }
                    false
                }
            }
        })
    })
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Truncate a string for UI payloads on a char boundary (tool-call cards).
fn truncate_for_ui(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}…")
}

/// State of one tracked async run (`/v1/runs`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunState {
    pub run_id: String,
    pub status: String,
    pub session_id: Option<String>,
    pub message: String,
    pub created_at: f64,
    pub finished_at: Option<f64>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub iterations: Option<usize>,
    pub stop_requested: bool,
    /// Pending (or last resolved) approval request for this run.
    pub approval: Option<Value>,
}

/// Shared gateway state.
/// Process-wide gateway counters for `GET /metrics` (Prometheus text
/// format). An ulnclaw ops extension — hermes' api_server has no metrics
/// endpoint.
#[derive(Debug, Default)]
pub struct GatewayMetrics {
    pub chat_completions: std::sync::atomic::AtomicU64,
    pub responses_requests: std::sync::atomic::AtomicU64,
    pub session_chats: std::sync::atomic::AtomicU64,
    pub runs_started: std::sync::atomic::AtomicU64,
    pub runs_completed: std::sync::atomic::AtomicU64,
    pub runs_failed: std::sync::atomic::AtomicU64,
    pub prompt_tokens: std::sync::atomic::AtomicU64,
    pub completion_tokens: std::sync::atomic::AtomicU64,
    pub tool_calls: std::sync::atomic::AtomicU64,
}

impl GatewayMetrics {
    fn add(counter: &std::sync::atomic::AtomicU64, value: u64) {
        if value > 0 {
            counter.fetch_add(value, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Accumulate usage + tool-call counts from one finished agent run.
    pub fn record_run(&self, usage: &crate::provider::Usage, tool_call_count: usize) {
        Self::add(&self.prompt_tokens, usage.prompt_tokens as u64);
        Self::add(&self.completion_tokens, usage.completion_tokens as u64);
        Self::add(&self.tool_calls, tool_call_count as u64);
    }
}

pub struct GatewayState {
    pub agent: Arc<Agent>,
    pub store: Arc<SqliteSessionStore>,
    pub model_name: String,
    pub provider_name: String,
    pub key: Option<String>,
    pub runs: Arc<Mutex<HashMap<String, RunState>>>,
    /// Stored `/v1/responses` objects, keyed by response id.
    pub responses: Arc<Mutex<HashMap<String, Value>>>,
    /// Approval routing for dangerous terminal commands.
    pub router: Arc<ApprovalRouter>,
    /// Unresolved approval responders, keyed by run id.
    pub pending_approvals: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>,
    /// Cron job store backing `/api/jobs` (set when the gateway owns a home dir).
    pub cron: OnceLock<Arc<CronStore>>,
    /// Skills directory backing `GET /v1/skills`.
    pub skills_dir: OnceLock<PathBuf>,
    /// Request/run counters backing `GET /metrics`.
    pub metrics: Arc<GatewayMetrics>,
    /// Gateway start instant (uptime gauge).
    pub started_at: std::time::Instant,
}

impl GatewayState {
    /// Build state from a wired agent (store must be attached).
    pub fn new(
        agent: Arc<Agent>,
        model_name: String,
        provider_name: String,
        key: Option<String>,
        router: Arc<ApprovalRouter>,
    ) -> Result<Arc<Self>> {
        let store = agent.store().ok_or_else(|| {
            AgentError::config("gateway requires the SQLite session store (agent.store())")
        })?;
        Ok(Arc::new(Self {
            agent,
            store,
            model_name,
            provider_name,
            key,
            runs: Arc::new(Mutex::new(HashMap::new())),
            responses: Arc::new(Mutex::new(HashMap::new())),
            router,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            cron: OnceLock::new(),
            skills_dir: OnceLock::new(),
            metrics: Arc::new(GatewayMetrics::default()),
            started_at: std::time::Instant::now(),
        }))
    }
}

/// `GET /metrics` — Prometheus text-format counters and gauges.
/// An ulnclaw ops extension (hermes' api_server has no metrics endpoint).
async fn metrics(State(state): State<Arc<GatewayState>>) -> Response {
    let uptime = state.started_at.elapsed().as_secs_f64();
    let sessions = state.store.count_sessions().unwrap_or(0);
    let messages = state.store.count_messages().unwrap_or(0);
    let active_runs = state.runs.lock().await.len();
    let mut cron_jobs_enabled = 0usize;
    let mut cron_jobs_disabled = 0usize;
    if let Some(cron) = state.cron.get() {
        if let Ok(jobs) = cron.list() {
            for job in &jobs {
                if job.enabled {
                    cron_jobs_enabled += 1;
                } else {
                    cron_jobs_disabled += 1;
                }
            }
        }
    }
    let m = &state.metrics;
    use std::sync::atomic::Ordering::Relaxed;
    let body = format!(
        "# HELP ulnclaw_uptime_seconds Gateway uptime in seconds.
# TYPE ulnclaw_uptime_seconds gauge
ulnclaw_uptime_seconds {uptime:.3}
# HELP ulnclaw_build_info Build metadata.
# TYPE ulnclaw_build_info gauge
ulnclaw_build_info{{version=\"{version}\",provider=\"{provider}\",model=\"{model}\"}} 1
# HELP ulnclaw_sessions_total Sessions stored.
# TYPE ulnclaw_sessions_total gauge
ulnclaw_sessions_total {sessions}
# HELP ulnclaw_messages_total Messages stored.
# TYPE ulnclaw_messages_total gauge
ulnclaw_messages_total {messages}
# HELP ulnclaw_runs_active Currently tracked runs.
# TYPE ulnclaw_runs_active gauge
ulnclaw_runs_active {active_runs}
# HELP ulnclaw_cron_jobs Cron jobs by state.
# TYPE ulnclaw_cron_jobs gauge
ulnclaw_cron_jobs{{state=\"enabled\"}} {cron_jobs_enabled}
ulnclaw_cron_jobs{{state=\"disabled\"}} {cron_jobs_disabled}
# HELP ulnclaw_http_requests_total Gateway HTTP requests by endpoint.
# TYPE ulnclaw_http_requests_total counter
ulnclaw_http_requests_total{{endpoint=\"chat_completions\"}} {chat_completions}
ulnclaw_http_requests_total{{endpoint=\"responses\"}} {responses_requests}
ulnclaw_http_requests_total{{endpoint=\"session_chat\"}} {session_chats}
# HELP ulnclaw_runs_total Tracked runs by outcome.
# TYPE ulnclaw_runs_total counter
ulnclaw_runs_total{{outcome=\"started\"}} {runs_started}
ulnclaw_runs_total{{outcome=\"completed\"}} {runs_completed}
ulnclaw_runs_total{{outcome=\"failed\"}} {runs_failed}
# HELP ulnclaw_tokens_total Tokens consumed by direction.
# TYPE ulnclaw_tokens_total counter
ulnclaw_tokens_total{{direction=\"prompt\"}} {prompt_tokens}
ulnclaw_tokens_total{{direction=\"completion\"}} {completion_tokens}
# HELP ulnclaw_tool_calls_total Tool calls executed via gateway runs.
# TYPE ulnclaw_tool_calls_total counter
ulnclaw_tool_calls_total {tool_calls}
",
        uptime = uptime,
        version = env!("CARGO_PKG_VERSION"),
        provider = state.provider_name,
        model = state.model_name,
        sessions = sessions,
        messages = messages,
        active_runs = active_runs,
        cron_jobs_enabled = cron_jobs_enabled,
        cron_jobs_disabled = cron_jobs_disabled,
        chat_completions = m.chat_completions.load(Relaxed),
        responses_requests = m.responses_requests.load(Relaxed),
        session_chats = m.session_chats.load(Relaxed),
        runs_started = m.runs_started.load(Relaxed),
        runs_completed = m.runs_completed.load(Relaxed),
        runs_failed = m.runs_failed.load(Relaxed),
        prompt_tokens = m.prompt_tokens.load(Relaxed),
        completion_tokens = m.completion_tokens.load(Relaxed),
        tool_calls = m.tool_calls.load(Relaxed),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

/// Query parameters for `GET /api/usage`.
#[derive(Debug, Deserialize)]
struct UsageQuery {
    /// Number of per-session rows to include (default 20, max 200).
    limit: Option<usize>,
}

/// `GET /api/usage` — token accounting across the gateway process
/// (since startup) and the session store (all time), plus the most
/// recent per-session rows. An ulnclaw ops extension — hermes'
/// api_server has no usage endpoint.
async fn usage(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<UsageQuery>,
) -> Json<Value> {
    use std::sync::atomic::Ordering::Relaxed;
    let m = &state.metrics;
    let prompt = m.prompt_tokens.load(Relaxed);
    let completion = m.completion_tokens.load(Relaxed);
    let (store_sessions, store_input, store_output) =
        state.store.token_totals().unwrap_or((0, 0, 0));
    let messages = state.store.count_messages().unwrap_or(0);
    let limit = query.limit.unwrap_or(20).min(200);
    let sessions: Vec<Value> = state
        .store
        .list_session_rows(limit)
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "source": row.source,
                "model": row.model,
                "title": row.title,
                "started_at": row.started_at,
                "ended_at": row.ended_at,
                "end_reason": row.end_reason,
                "message_count": row.message_count,
                "input_tokens": row.input_tokens,
                "output_tokens": row.output_tokens,
                "total_tokens": row.input_tokens + row.output_tokens,
            })
        })
        .collect();
    Json(json!({
        "process": {
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "total_tokens": prompt + completion,
            "tool_calls": m.tool_calls.load(Relaxed),
            "requests": {
                "chat_completions": m.chat_completions.load(Relaxed),
                "responses": m.responses_requests.load(Relaxed),
                "session_chats": m.session_chats.load(Relaxed),
            },
            "runs": {
                "started": m.runs_started.load(Relaxed),
                "completed": m.runs_completed.load(Relaxed),
                "failed": m.runs_failed.load(Relaxed),
            },
        },
        "store": {
            "sessions": store_sessions,
            "messages": messages,
            "input_tokens": store_input,
            "output_tokens": store_output,
            "total_tokens": store_input + store_output,
        },
        "sessions": sessions,
    }))
}

// ── Config editor API (desktop Config view) ───────────────────────────────

/// Placeholder the GET response uses for secret-looking values; PUT ignores
/// writes that still carry it (so a round-trip never clobbers secrets).
const CONFIG_REDACTED: &str = "[redacted]";

fn looks_like_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    ["key", "token", "secret", "password", "passwd", "credential"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Recursive serde_json → toml::Value conversion (toml 0.8 has no
/// TryFrom<serde_json::Value>); objects must stay string-keyed tables.
fn json_to_toml(value: Value) -> std::result::Result<toml::Value, String> {
    Ok(match value {
        Value::Null => toml::Value::String(String::new()),
        Value::Bool(b) => toml::Value::Boolean(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                return Err(format!("number {n} is not representable in TOML"));
            }
        }
        Value::String(t) => toml::Value::String(t),
        Value::Array(items) => toml::Value::Array(
            items
                .into_iter()
                .map(json_to_toml)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ),
        Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                table.insert(k, json_to_toml(v)?);
            }
            toml::Value::Table(table)
        }
    })
}

/// Redact secret-looking string leaves in place; returns the list of
/// dotted paths that were masked.
fn redact_config_value(value: &mut Value, prefix: &str, redacted: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                redact_config_value(v, &path, redacted);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter_mut().enumerate() {
                redact_config_value(v, &format!("{prefix}[{i}]"), redacted);
            }
        }
        Value::String(text) if looks_like_secret_key(prefix.rsplit(['.', '[']).next().unwrap_or(prefix)) => {
            if !text.is_empty() {
                *text = CONFIG_REDACTED.to_string();
                redacted.push(prefix.to_string());
            }
        }
        _ => {}
    }
}

/// `GET /api/config` — config.toml as nested JSON with secret-looking
/// leaves redacted, plus the .env key names present on disk (names only,
/// never values). Desktop Config view backing endpoint (ulnclaw extension).
async fn config_get() -> Response {
    let path = crate::config_cmd::config_path();
    let toml_value = match crate::config_cmd::load_toml(&path) {
        Ok(v) => v,
        Err(e) => return server_error(&e),
    };
    let mut json_value = serde_json::to_value(&toml_value).unwrap_or(Value::Null);
    let mut redacted = Vec::new();
    redact_config_value(&mut json_value, "", &mut redacted);
    let env_keys: Vec<String> = crate::dump::dotenv_key_names(&crate::config::ulnclaw_home())
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    Json(json!({
        "path": path.display().to_string(),
        "config": json_value,
        "redacted": redacted,
        "env_keys": env_keys,
        "note": "edits apply to new CLI/gateway processes; restart the gateway to apply here",
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ConfigPutBody {
    #[serde(default)]
    set: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    unset: Vec<String>,
}

/// `PUT /api/config` — apply dotted-path sets/unsets to config.toml.
/// Values equal to the redaction placeholder are skipped so a GET→PUT
/// round-trip never overwrites real secrets.
async fn config_put(Json(body): Json<ConfigPutBody>) -> Response {
    let path = crate::config_cmd::config_path();
    let mut toml_value = match crate::config_cmd::load_toml(&path) {
        Ok(v) => v,
        Err(e) => return server_error(&e),
    };
    let mut applied: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (key, value) in body.set {
        if key.trim().is_empty() {
            continue;
        }
        if value == Value::String(CONFIG_REDACTED.to_string()) {
            skipped.push(key);
            continue;
        }
        let toml_scalar = match json_to_toml(Value::clone(&value)) {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("value for '{key}' is not representable in TOML")})),
                )
                    .into_response()
            }
        };
        if let Err(e) = crate::config_cmd::set_nested(&mut toml_value, &key, toml_scalar) {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response();
        }
        applied.push(key);
    }
    for key in &body.unset {
        crate::config_cmd::unset_nested(&mut toml_value, key);
        applied.push(format!("-{key}"));
    }
    if let Err(e) = crate::config_cmd::save_toml(&path, &toml_value) {
        return server_error(&e);
    }
    Json(json!({
        "ok": true,
        "applied": applied,
        "skipped_redacted": skipped,
        "path": path.display().to_string(),
        "note": "restart the gateway to apply changes to the running process",
    }))
    .into_response()
}

/// Query parameters for `GET /api/doctor`.
#[derive(Debug, Deserialize)]
struct DoctorQuery {
    /// Run provider connectivity probes (hermes `doctor --online`); off by
    /// default so the endpoint stays fast.
    online: Option<bool>,
}

/// `GET /api/doctor` — run the doctor report (same checks as
/// `ulnclaw doctor`) and return it as JSON for the desktop Doctor view
/// (ulnclaw extension; hermes has no HTTP doctor surface).
async fn doctor_report(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<DoctorQuery>,
) -> Response {
    let config = state.agent.context().config.clone();
    let online = query.online.unwrap_or(false);
    let report = tokio::task::spawn_blocking(move || {
        crate::doctor::run_doctor(
            &config,
            &crate::doctor::DoctorOptions {
                fix: false,
                online,
                json: true,
            },
        )
    })
    .await
    .unwrap_or_default();
    Json(json!({
        "report": serde_json::to_value(&report).unwrap_or(Value::Null),
        "online": online,
    }))
    .into_response()
}

/// `GET /api/monitoring` — structured view of `ulnclaw monitoring status`:
/// export posture + OTLP destination + emitter queue depth (content-free
/// by design; desktop Doctor view monitoring panel).
async fn monitoring_status(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let config = &state.agent.context().config.monitoring;
    let endpoint = config
        .export
        .otlp
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string);
    Json(json!({
        "enabled": config.enabled(),
        "metrics": config.metrics_enabled(),
        "metrics_interval_seconds": config.export_interval_seconds(),
        "diagnostic_events": config.diagnostic_events_enabled(),
        "warning_error_logs": config.warning_error_events_enabled(),
        "logs_interval_seconds": config.logs_export_interval_seconds(),
        "otlp": {
            "enabled": config.otlp_enabled(),
            "endpoint": endpoint,
            "transport": "otlp/http-json",
        },
        "install_id": config.install_id,
        "queue_depth": crate::monitoring::queue_len_estimate(),
        "scope": "gateway service health + redacted diagnostics only;                   prompts, messages, tool args/results and usage are never exported",
    }))
}

/// Query parameters for `GET /api/logs/tail`.
#[derive(Debug, Deserialize)]
struct LogsTailQuery {
    /// Number of trailing lines to return (default 200, max 1000).
    lines: Option<usize>,
    /// Minimum level filter (`debug`/`info`/`warn`/`error`).
    level: Option<String>,
}

/// `GET /api/logs/tail` — tail of `gateway.log` (same source as
/// `ulnclaw logs tail gateway`), optional min-level filter; desktop
/// Doctor view logs panel (ulnclaw extension).
async fn logs_tail(Query(query): Query<LogsTailQuery>) -> Response {
    let path = crate::logs::logs_dir().join("gateway.log");
    let num_lines = query.lines.unwrap_or(200).min(1000);
    let filters = crate::logs::LogFilters {
        min_level: query
            .level
            .filter(|level| !level.trim().is_empty())
            .map(|level| level.trim().to_uppercase()),
        session: None,
        since: None,
        component_prefixes: None,
    };
    match crate::logs::read_tail(&path, num_lines, &filters) {
        Ok(tail) => Json(json!({
            "path": path.display().to_string(),
            "lines": tail,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("gateway.log unavailable: {e}")})),
        )
            .into_response(),
    }
}

/// `GET /api/mcp/servers` — configured MCP servers with transport kind
/// and auth posture (static headers vs OAuth with stored tokens); desktop
/// Doctor view MCP panel (ulnclaw extension).
async fn mcp_servers_list(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let context = state.agent.context();
    let home = context.home.clone();
    let servers: Vec<Value> = context
        .config
        .mcp
        .servers
        .iter()
        .map(|server| {
            let kind = if server.url.is_some() {
                if server.transport.as_deref() == Some("sse") {
                    "sse"
                } else {
                    "http"
                }
            } else {
                "stdio"
            };
            let target = if let Some(url) = &server.url {
                url.clone()
            } else {
                let mut command = server.command.clone();
                for arg in &server.args {
                    command.push(' ');
                    command.push_str(arg);
                }
                command
            };
            let auth = if server.auth.as_deref() == Some("oauth") {
                "oauth"
            } else if !server.headers.is_empty() {
                "headers"
            } else {
                "none"
            };
            let oauth_tokens = server.auth.as_deref() == Some("oauth")
                && crate::mcp::oauth::load_tokens(&home, &server.name).is_some();
            // Discovered tools from the schema cache (fingerprint-gated).
            let fingerprint = crate::mcp::schema_cache::config_fingerprint(server);
            let cached_tools: Vec<Value> =
                crate::mcp::schema_cache::get_cached_entry_in(&home, &server.name, &fingerprint)
                    .map(|entry| {
                        crate::mcp::schema_cache::tools_from_cache_entry(&entry)
                            .into_iter()
                            .map(|tool| {
                                json!({
                                    "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                    "description": tool
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(""),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            json!({
                "name": server.name,
                "kind": kind,
                "target": target,
                "auth": auth,
                "oauth_tokens": oauth_tokens,
                "cached_tools": cached_tools,
            })
        })
        .collect();
    Json(json!({ "servers": servers }))
}

/// Query parameters for `GET /api/sessions/search`.
#[derive(Debug, Deserialize)]
struct SessionSearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/sessions/search?q=...` — full-text search across all session
/// transcripts (FTS5 with LIKE fallback), returning session ids with
/// snippets and titles (desktop Sessions view search; hermes
/// `hermes_state_search` parity).
async fn search_sessions(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<SessionSearchQuery>,
) -> Response {
    let Some(q) = query.q.map(|q| q.trim().to_string()).filter(|q| !q.is_empty()) else {
        return bad_request("q is required", Some("invalid_request"));
    };
    let limit = query.limit.unwrap_or(30).min(200).max(1);
    let store = state.store.clone();
    let result =
        tokio::task::spawn_blocking(move || -> std::result::Result<Value, crate::error::AgentError> {
            let hits = store.search_messages(&q, limit)?;
            let results: Vec<Value> = hits
                .into_iter()
                .map(|(session_id, snippet)| {
                    let title = store.get_session_title(&session_id).ok().flatten();
                    json!({
                        "session_id": session_id,
                        "title": title,
                        "snippet": snippet,
                    })
                })
                .collect();
            Ok(json!({"query": q, "count": results.len(), "results": results}))
        })
        .await;
    match result {
        Ok(Ok(payload)) => Json(payload).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("search task failed: {e}")})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
struct SessionPruneBody {
    #[serde(default)]
    older_than: Option<String>,
    #[serde(default)]
    newer_than: Option<String>,
    #[serde(default)]
    before: Option<String>,
    #[serde(default)]
    after: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    end_reason: Option<String>,
    #[serde(default)]
    include_archived: bool,
    #[serde(default)]
    dry_run: bool,
}

fn build_prune_filters(
    body: &SessionPruneBody,
) -> std::result::Result<crate::session::filters::PruneFilters, String> {
    use crate::session::filters::{parse_point_in_time, PruneFilters};
    let mut filters = PruneFilters::default();
    if let Some(value) = body.older_than.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        filters.last_active_before = Some(parse_point_in_time(value, "older_than")?);
    }
    if let Some(value) = body.newer_than.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        filters.last_active_after = Some(parse_point_in_time(value, "newer_than")?);
    }
    if let Some(value) = body.before.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        filters.started_before = Some(parse_point_in_time(value, "before")?);
    }
    if let Some(value) = body.after.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        filters.started_after = Some(parse_point_in_time(value, "after")?);
    }
    filters.source = body.source.clone().filter(|v| !v.trim().is_empty());
    filters.title_like = body.title.clone().filter(|v| !v.trim().is_empty());
    filters.end_reason = body.end_reason.clone().filter(|v| !v.trim().is_empty());
    Ok(filters)
}

/// Shared prune/archive worker mirroring the CLI `run_session_prune`:
/// dry-run previews the candidate list, otherwise applies and reports
/// the affected count.
async fn apply_session_prune(
    state: Arc<GatewayState>,
    filters: crate::session::filters::PruneFilters,
    delete: bool,
    dry_run: bool,
) -> Response {
    let store = state.store.clone();
    let result =
        tokio::task::spawn_blocking(move || -> std::result::Result<Value, crate::error::AgentError> {
            let candidates = store.list_prune_candidates(&filters)?;
            if dry_run {
                let rows: Vec<Value> = candidates
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "title": c.title,
                            "source": c.source,
                            "model": c.model,
                            "message_count": c.message_count,
                            "last_active": c.last_active,
                            "archived": c.archived,
                        })
                    })
                    .collect();
                return Ok(json!({
                    "dry_run": true,
                    "count": candidates.len(),
                    "describe": filters.describe(),
                    "candidates": rows,
                }));
            }
            let affected = if delete {
                store.prune_sessions(&filters)?
            } else {
                store.archive_sessions(&filters)?
            };
            Ok(json!({
                "dry_run": false,
                "affected": affected,
                "describe": filters.describe(),
            }))
        })
        .await;
    match result {
        Ok(Ok(payload)) => Json(payload).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("prune task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `POST /api/sessions/prune` — delete ended sessions matching filters
/// (mirrors `ulnclaw sessions prune`). A bare request with no filters
/// applies hermes' implicit "older than 90 days" cutoff; archived
/// sessions are skipped unless `include_archived`. `dry_run: true`
/// previews the candidates without deleting anything.
async fn prune_sessions(State(state): State<Arc<GatewayState>>, Json(body): Json<SessionPruneBody>) -> Response {
    let mut filters = match build_prune_filters(&body) {
        Ok(f) => f,
        Err(e) => return bad_request(&e, Some("invalid_request")),
    };
    if filters.is_empty() {
        let cutoff = match crate::session::filters::parse_point_in_time("90", "older_than") {
            Ok(v) => v,
            Err(e) => return bad_request(&e, Some("invalid_request")),
        };
        filters.last_active_before = Some(cutoff);
    }
    filters.archived = if body.include_archived { None } else { Some(false) };
    apply_session_prune(state, filters, true, body.dry_run).await
}

/// `POST /api/sessions/archive` — soft-hide ended sessions matching
/// filters (mirrors `ulnclaw sessions archive`; recoverable — nothing
/// is deleted). Refuses filter-less requests so the whole store cannot
/// be archived by accident. `dry_run: true` previews.
async fn archive_sessions(State(state): State<Arc<GatewayState>>, Json(body): Json<SessionPruneBody>) -> Response {
    let mut filters = match build_prune_filters(&body) {
        Ok(f) => f,
        Err(e) => return bad_request(&e, Some("invalid_request")),
    };
    if filters.is_empty() {
        return bad_request(
            "Refusing to archive every ended session: pass at least one filter (e.g. older_than: \"30d\", source: \"cli\").",
            Some("invalid_request"),
        );
    }
    filters.archived = Some(false);
    apply_session_prune(state, filters, false, body.dry_run).await
}

/// Query parameters for `GET /api/insights`.
#[derive(Debug, Deserialize)]
struct InsightsQuery {
    /// Analysis window in days (default 30, max 365).
    days: Option<u32>,
    /// Filter to one session source (cli, telegram, …).
    source: Option<String>,
}

/// `GET /api/insights` — usage analytics over the session store (same
/// engine as `ulnclaw insights`); desktop Usage view insights section
/// (ulnclaw extension).
async fn insights(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<InsightsQuery>,
) -> Response {
    let days = query.days.unwrap_or(30).min(365).max(1);
    let source = query.source.filter(|src| !src.trim().is_empty());
    let provider_hint = state.agent.context().config.model.provider.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = crate::insights::InsightsEngine::open_default()?;
        engine.generate(days, source.as_deref(), Some(&provider_hint))
    })
    .await
    .unwrap_or_else(|e| Err(crate::error::AgentError::Tool(format!("insights task failed: {e}"))));
    match result {
        Ok(report) => Json(serde_json::to_value(&report).unwrap_or(Value::Null)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `GET /api/channels` — messaging-platform inventory: every known
/// platform with its `[messaging.<platform>].enabled` posture (desktop
/// Doctor channels panel; hermes ChannelsPage parity).
async fn channels_status(State(_state): State<Arc<GatewayState>>) -> Response {
    let result = tokio::task::spawn_blocking(|| {
        let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
        let m = &config.messaging;
        let channels: Vec<(&str, bool)> = vec![
            ("telegram", m.telegram.enabled),
            ("discord", m.discord.enabled),
            ("slack", m.slack.enabled),
            ("signal", m.signal.enabled),
            ("weixin", m.weixin.enabled),
            ("qq", m.qq.enabled),
            ("yuanbao", m.yuanbao.enabled),
            ("email", m.email.enabled),
            ("mattermost", m.mattermost.enabled),
            ("matrix", m.matrix.enabled),
            ("dingtalk", m.dingtalk.enabled),
            ("wecom", m.wecom.enabled),
            ("feishu", m.feishu.enabled),
            ("homeassistant", m.homeassistant.enabled),
            ("sms", m.sms.enabled),
            ("whatsapp", m.whatsapp.enabled),
            ("irc", m.irc.enabled),
            ("ntfy", m.ntfy.enabled),
            ("simplex", m.simplex.enabled),
            ("teams", m.teams.enabled),
            ("line", m.line.enabled),
            ("google_chat", m.google_chat.enabled),
            ("buzz", m.buzz.enabled),
            ("photon", m.photon.enabled),
            ("raft", m.raft.enabled),
            ("a2a", m.a2a.enabled),
        ];
        let rows: Vec<Value> = channels
            .into_iter()
            .map(|(name, enabled)| json!({"name": name, "enabled": enabled}))
            .collect();
        let enabled_count = rows.iter().filter(|r| r["enabled"] == Value::Bool(true)).count();
        json!({"channels": rows, "enabled_count": enabled_count})
    })
    .await;
    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("channels task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `GET /api/egress/status` — egress-proxy status text (same
/// `format_status_text` the `/egress` slash command and CLI print;
/// tokens always redacted). Desktop Doctor egress panel.
async fn egress_status(State(_state): State<Arc<GatewayState>>) -> Response {
    let result = tokio::task::spawn_blocking(|| crate::egress_cmd::format_status_text(false)).await;
    match result {
        Ok(text) => Json(json!({"text": text})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("egress status task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `GET /api/system` — gateway/system facts for the desktop Doctor
/// system panel: version, platform, home/config paths, uptime, process
/// id and store/run/cron/plugins counts (ulnclaw extension; hermes
/// SystemPage parity).
async fn system_info(State(state): State<Arc<GatewayState>>) -> Response {
    let uptime_secs = state.started_at.elapsed().as_secs();
    let sessions = state.store.count_sessions().unwrap_or(0);
    let messages = state.store.count_messages().unwrap_or(0);
    let active_runs = state.runs.lock().await.len();
    let mut cron_jobs_enabled = 0usize;
    let mut cron_jobs_disabled = 0usize;
    if let Some(cron) = state.cron.get() {
        if let Ok(jobs) = cron.list() {
            for job in &jobs {
                if job.enabled {
                    cron_jobs_enabled += 1;
                } else {
                    cron_jobs_disabled += 1;
                }
            }
        }
    }
    let plugins_loaded = crate::plugins::loaded_plugins().len();
    let payload = tokio::task::spawn_blocking(move || {
        let home = crate::config::ulnclaw_home();
        json!({
            "service": "ulnclaw-gateway",
            "version": crate::VERSION,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "home": home.to_string_lossy(),
            "config_path": home.join("config.toml").to_string_lossy(),
            "pid": std::process::id(),
            "uptime_secs": uptime_secs,
            "desktop_managed": std::env::var("ULNCLAW_DESKTOP").is_ok(),
            "sessions": sessions,
            "messages": messages,
            "active_runs": active_runs,
            "cron_jobs_enabled": cron_jobs_enabled,
            "cron_jobs_disabled": cron_jobs_disabled,
            "plugins_loaded": plugins_loaded,
        })
    })
    .await
    .unwrap_or_else(|e| json!({"error": format!("system task failed: {e}")}));
    Json(payload).into_response()
}

/// Body for the pairing mutation endpoints.
#[derive(Debug, Deserialize)]
struct PairingActionBody {
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

/// `GET /api/pairing` — pending + approved pairings per platform with
/// lockout state (desktop Pairing view; hermes `pairing list` parity).
async fn pairing_status(State(_state): State<Arc<GatewayState>>) -> Response {
    let result = tokio::task::spawn_blocking(|| {
        let home = crate::config::ulnclaw_home();
        let store = crate::pairing::PairingStore::open(&home);
        let mut platforms = Vec::new();
        for platform in store.known_platforms() {
            let pending: Vec<Value> = store
                .list_pending(&platform)
                .iter()
                .map(|request| {
                    json!({
                        "request_id": request.request_id,
                        "user_id": request.user_id,
                        "user_name": request.user_name,
                        "age_minutes": request.age_minutes,
                    })
                })
                .collect();
            let approved: Vec<Value> = store
                .list_approved(&platform)
                .iter()
                .map(|grant| {
                    json!({
                        "user_id": grant.user_id,
                        "user_name": grant.user_name,
                    })
                })
                .collect();
            platforms.push(json!({
                "platform": platform,
                "locked_out": store.is_locked_out(&platform),
                "pending": pending,
                "approved": approved,
            }));
        }
        json!({"platforms": platforms})
    })
    .await;
    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("pairing task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `POST /api/pairing/approve` — approve a pending pairing code
/// (hermes `pairing approve <platform> <code>`).
async fn pairing_approve(
    State(_state): State<Arc<GatewayState>>,
    Json(body): Json<PairingActionBody>,
) -> Response {
    let Some(platform) = body.platform.filter(|p| !p.trim().is_empty()) else {
        return bad_request("platform is required", Some("invalid_request"));
    };
    let Some(code) = body.code.filter(|c| !c.trim().is_empty()) else {
        return bad_request("code is required", Some("invalid_request"));
    };
    let result = tokio::task::spawn_blocking({
        let platform = platform.clone();
        let code = code.clone();
        move || {
            let home = crate::config::ulnclaw_home();
            let store = crate::pairing::PairingStore::open(&home);
            store.approve_code(&platform, &code)
        }
    })
    .await
    .unwrap_or(None);
    match result {
        Some(grant) => Json(json!({
            "ok": true,
            "platform": platform,
            "user_id": grant.user_id,
            "user_name": grant.user_name,
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("no pending pairing matching {code} on {platform}")})),
        )
            .into_response(),
    }
}

/// `POST /api/pairing/revoke` — revoke an approved pairing (hermes
/// `pairing revoke <platform> <user-id>`).
async fn pairing_revoke(
    State(_state): State<Arc<GatewayState>>,
    Json(body): Json<PairingActionBody>,
) -> Response {
    let Some(platform) = body.platform.filter(|p| !p.trim().is_empty()) else {
        return bad_request("platform is required", Some("invalid_request"));
    };
    let Some(user_id) = body.user_id.filter(|u| !u.trim().is_empty()) else {
        return bad_request("user_id is required", Some("invalid_request"));
    };
    let revoked = tokio::task::spawn_blocking({
        let platform = platform.clone();
        let user_id = user_id.clone();
        move || {
            let home = crate::config::ulnclaw_home();
            let store = crate::pairing::PairingStore::open(&home);
            store.revoke(&platform, &user_id)
        }
    })
    .await
    .unwrap_or(false);
    if revoked {
        Json(json!({"ok": true, "platform": platform, "user_id": user_id})).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("{user_id} is not paired on {platform}")})),
        )
            .into_response()
    }
}

/// `POST /api/pairing/clear-pending` — drop pending codes for one
/// platform, or all known platforms when omitted (hermes `pairing
/// clear-pending`).
async fn pairing_clear_pending(
    State(_state): State<Arc<GatewayState>>,
    Json(body): Json<PairingActionBody>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let home = crate::config::ulnclaw_home();
        let store = crate::pairing::PairingStore::open(&home);
        let targets: Vec<String> = match body.platform.filter(|p| !p.trim().is_empty()) {
            Some(platform) => vec![platform],
            None => {
                let mut all = store.known_platforms();
                if all.is_empty() {
                    all = vec!["telegram".into(), "discord".into(), "slack".into()];
                }
                all
            }
        };
        let mut cleared = 0usize;
        for platform in &targets {
            cleared += store.clear_pending(platform);
        }
        cleared
    })
    .await;
    match result {
        Ok(cleared) => Json(json!({"ok": true, "cleared": cleared})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("clear-pending task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `GET /api/plugins` — plugin inventory: loaded plugins (manifest,
/// disabled flag, dir), config shell hooks (`[hooks]`) and the disabled
/// list (desktop Plugins view; hermes `plugins list` parity).
async fn plugins_inventory(State(_state): State<Arc<GatewayState>>) -> Response {
    let result = tokio::task::spawn_blocking(|| {
        let home = crate::config::ulnclaw_home();
        let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
        let plugins: Vec<Value> = crate::plugins::loaded_plugins()
            .iter()
            .map(|plugin| {
                json!({
                    "name": plugin.manifest.name,
                    "version": plugin.manifest.version,
                    "description": plugin.manifest.description,
                    "hooks": plugin.manifest.hooks,
                    "tools": plugin
                        .manifest
                        .tools
                        .iter()
                        .map(|tool| {
                            json!({
                                "name": tool.name,
                                "description": tool.description,
                            })
                        })
                        .collect::<Vec<_>>(),
                    "disabled": plugin.disabled,
                    "dir": plugin.dir.to_string_lossy(),
                })
            })
            .collect();
        let mut config_hooks = serde_json::Map::new();
        for (event, commands) in &config.hooks.events {
            config_hooks.insert(event.clone(), json!(commands));
        }
        json!({
            "plugins": plugins,
            "config_hooks": Value::Object(config_hooks),
            "disabled": crate::plugins::current_disabled(&home),
        })
    })
    .await;
    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("plugins task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `POST /api/plugins/:name/enable` — remove the plugin from the config
/// deny-list (hermes `plugins enable`).
async fn plugin_enable(State(_state): State<Arc<GatewayState>>, Path(name): Path<String>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let home = crate::config::ulnclaw_home();
        crate::plugins::enable_plugin(&home, &name)
    })
    .await
    .unwrap_or_else(|e| Err(format!("enable task failed: {e}")));
    match result {
        Ok(message) => Json(json!({"ok": true, "message": message})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

/// `POST /api/plugins/:name/disable` — add the plugin to the config
/// deny-list (hermes `plugins disable`).
async fn plugin_disable(
    State(_state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let home = crate::config::ulnclaw_home();
        crate::plugins::disable_plugin(&home, &name)
    })
    .await
    .unwrap_or_else(|e| Err(format!("disable task failed: {e}")));
    match result {
        Ok(message) => Json(json!({"ok": true, "message": message})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

/// `GET /api/storage` — session-store footprint: logical database size,
/// WAL size, session/message counts and the on-disk path (desktop Doctor
/// storage panel; ulnclaw extension).
async fn storage_status(State(state): State<Arc<GatewayState>>) -> Response {
    let store = state.store.clone();
    let stats = tokio::task::spawn_blocking(move || {
        let home = crate::config::ulnclaw_home();
        let db_path = home.join("state.db");
        let file_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        let size_bytes = store.logical_size_bytes().unwrap_or(file_bytes);
        let wal_bytes = std::fs::metadata(home.join("state.db-wal"))
            .map(|m| m.len())
            .unwrap_or(0);
        json!({
            "db_path": db_path.to_string_lossy(),
            "size_bytes": size_bytes,
            "wal_bytes": wal_bytes,
            "sessions": store.count_sessions().unwrap_or(0),
            "messages": store.count_messages().unwrap_or(0),
        })
    })
    .await
    .unwrap_or_else(|e| json!({"error": format!("storage task failed: {e}")}));
    Json(stats).into_response()
}

/// `POST /api/storage/optimize` — FTS segment merge + WAL checkpoint +
/// VACUUM over the session store (same work as `ulnclaw sessions
/// optimize`; desktop Doctor storage panel).
async fn storage_optimize(State(state): State<Arc<GatewayState>>) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let before_bytes = store.logical_size_bytes().unwrap_or(0);
        let merged = store.optimize_storage()?;
        let after_bytes = store.logical_size_bytes().unwrap_or(before_bytes);
        Ok::<Value, crate::error::AgentError>(json!({
            "merged_indexes": merged,
            "before_bytes": before_bytes,
            "after_bytes": after_bytes,
        }))
    })
    .await
    .unwrap_or_else(|e| {
        Err(crate::error::AgentError::Tool(format!(
            "optimize task failed: {e}"
        )))
    });
    match result {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Build the HTTP router (also used by tests).
pub fn router(state: Arc<GatewayState>) -> Router {
    let router = Router::new()
        .route("/health", get(health))
        .route("/health/detailed", get(health_detailed))
        .route("/v1/health", get(health))
        .route("/v1/models", get(models))
        .route("/api/model/options", get(model_options))
        .route("/v1/capabilities", get(capabilities))
        .route("/metrics", get(metrics))
        .route("/api/usage", get(usage))
        .route("/api/config", get(config_get).put(config_put))
        .route("/api/doctor", get(doctor_report))
        .route("/api/monitoring", get(monitoring_status))
        .route("/api/logs/tail", get(logs_tail))
        .route("/api/mcp/servers", get(mcp_servers_list))
        .route("/api/insights", get(insights))
        .route("/api/channels", get(channels_status))
        .route("/api/egress/status", get(egress_status))
        .route("/api/system", get(system_info))
        .route("/api/pairing", get(pairing_status))
        .route("/api/pairing/approve", post(pairing_approve))
        .route("/api/pairing/revoke", post(pairing_revoke))
        .route("/api/pairing/clear-pending", post(pairing_clear_pending))
        .route("/api/plugins", get(plugins_inventory))
        .route("/api/plugins/:name/enable", post(plugin_enable))
        .route("/api/plugins/:name/disable", post(plugin_disable))
        .route("/api/storage", get(storage_status))
        .route("/api/storage/optimize", post(storage_optimize))
        .route("/api/backups", get(list_backups).post(create_backup))
        .route("/api/backups/prune", post(prune_backups))
        .route("/api/backups/:id/restore", post(restore_backup))
        .route("/api/curator", get(curator_status))
        .route("/api/curator/pin", post(curator_pin))
        .route("/api/curator/unpin", post(curator_unpin))
        .route("/api/curator/archive", post(curator_archive))
        .route("/api/curator/restore", post(curator_restore))
        .route("/api/checkpoints/status", get(checkpoints_status))
        .route("/api/checkpoints", get(checkpoints_list))
        .route("/api/checkpoints/restore", post(checkpoints_restore))
        .route("/api/checkpoints/prune", post(checkpoints_prune))
        .route("/api/webhooks/subscriptions", get(webhook_subscriptions_list).post(webhook_subscriptions_create))
        .route("/api/webhooks/subscriptions/:name", delete(webhook_subscriptions_delete))
        .route("/api/webhooks/subscriptions/:name/test", post(webhook_subscriptions_test))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(create_response))
        .route(
            "/v1/responses/:id",
            get(get_response).delete(delete_response),
        )
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/search", get(search_sessions))
        .route("/api/sessions/prune", post(prune_sessions))
        .route("/api/sessions/archive", post(archive_sessions))
        .route(
            "/api/sessions/:id",
            get(get_session).patch(patch_session).delete(delete_session),
        )
        .route("/api/sessions/:id/fork", post(fork_session))
        .route("/api/sessions/:id/messages", get(session_messages))
        .route("/api/sessions/:id/export", get(export_session))
        .route("/api/sessions/:id/chat", post(session_chat))
        .route("/api/sessions/:id/chat/stream", post(session_chat_stream))
        .route("/api/sessions/:id/model", post(lock_session_model))
        .route("/api/sessions/:id/recap", get(session_recap))
        .route("/api/uploads", post(upload_media))
        .route("/api/learning/graph", get(learning_graph))
        .route(
            "/api/learning/node",
            get(learning_node_get)
                .put(learning_node_put)
                .delete(learning_node_delete),
        )
        .route("/api/jobs", get(list_jobs).post(create_job))
        .route("/api/jobs/delivery-targets", get(job_delivery_targets))
        .route("/api/jobs/fire", post(fire_job))
        .route(
            "/api/jobs/:id",
            get(get_job).patch(update_job).delete(delete_job),
        )
        .route("/api/pets/config", get(pets::config))
        .route("/api/pets", get(pets::list))
        .route("/api/pets/:slug/spritesheet", get(pets::spritesheet))
        .route("/api/pets/hatch", post(pets::start_hatch))
        .route("/api/pets/hatch/:id", get(pets::hatch_status))
        .route("/api/pets/hatch/:id/pick", post(pets::pick_draft))
        .route("/api/pets/hatch/:id/cancel", post(pets::cancel_hatch))
        .route(
            "/api/pets/hatch/:id/draft/:index",
            get(pets::draft_image),
        )
        .route(
            "/api/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/api/projects/active", post(projects::set_active))
        .route("/api/projects/repos", get(projects::list_repos))
        .route("/api/projects/scan", post(projects::scan_repos))
        .route(
            "/api/projects/:id",
            get(projects::get_project)
                .patch(projects::update_project)
                .delete(projects::delete_project),
        )
        .route(
            "/api/projects/:id/folders",
            post(projects::add_folder).delete(projects::remove_folder),
        )
        .route("/api/projects/:id/primary", post(projects::set_primary))
        .route("/api/projects/:id/archive", post(projects::archive_project))
        .route("/api/projects/:id/restore", post(projects::restore_project))
        .route("/api/kanban/boards", get(kanban::list_boards).post(kanban::create_board))
        .route("/api/kanban/boards/:slug/switch", post(kanban::switch_board))
        .route("/api/kanban/tasks", get(kanban::list_tasks).post(kanban::create_task))
        .route("/api/kanban/dispatch", post(kanban::dispatch))
        .route("/api/kanban/tasks/:id", get(kanban::get_task))
        .route("/api/kanban/tasks/:id/complete", post(kanban::complete_task))
        .route("/api/kanban/tasks/:id/block", post(kanban::block_task))
        .route("/api/kanban/tasks/:id/unblock", post(kanban::unblock_task))
        .route("/api/kanban/tasks/:id/comment", post(kanban::comment_task))
        .route("/api/kanban/tasks/:id/link", post(kanban::link_task))
        .route("/api/kanban/tasks/:id/claim", post(kanban::claim_task))
        .route("/api/jobs/:id/pause", post(pause_job))
        .route("/api/jobs/:id/resume", post(resume_job))
        .route("/api/jobs/:id/run", post(run_job_now))
        .route("/v1/skills", get(skills_list))
        .route("/v1/toolsets", get(toolsets_list))
        .route("/v1/delegations", get(list_delegations_http))
        .route("/v1/delegations/:id", get(get_delegation_http))
        .route("/api/desktop/events", get(desktop_events))
        .route("/api/desktop/read-response", post(desktop_read_response))
        .route("/v1/browser/status", get(browser_status))
        .route("/v1/browser/connect", post(browser_connect))
        .route("/v1/browser/disconnect", post(browser_disconnect))
        .route("/v1/runs", get(list_runs).post(start_run))
        .route("/v1/runs/:id", get(get_run))
        .route("/v1/runs/:id/events", get(run_events))
        .route("/v1/runs/:id/approval", post(resolve_approval))
        .route("/v1/runs/:id/stop", post(stop_run))
        .route("/api/mcp/servers/:name/auth", post(mcp_server_auth))
        .route("/api/mcp/oauth/flows/:flow_id", get(mcp_oauth_flow_status))
        .route("/api/mcp/oauth/callback/:server_name", get(mcp_oauth_callback));
    let router = attach_webhook_routes(router, &state);
    router
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

/// Mount enabled webhook-platform ingress routes (hermes platform webhook
/// servers run inside the gateway HTTP surface).
fn attach_webhook_routes(
    router: Router<Arc<GatewayState>>,
    state: &Arc<GatewayState>,
) -> Router<Arc<GatewayState>> {
    let config = &state.agent.context().config;
    let mut router = router;
    if config.messaging.whatsapp_cloud.enabled {
        router = router.route(
            "/webhooks/whatsapp",
            get(whatsapp_verify_route).post(whatsapp_webhook_route),
        );
        tracing::info!("gateway webhook route mounted: /webhooks/whatsapp");
    }
    if config.messaging.msgraph.enabled {
        router = router.route(
            "/webhooks/msgraph",
            get(msgraph_webhook_route).post(msgraph_webhook_route),
        );
        tracing::info!("gateway webhook route mounted: /webhooks/msgraph");
    }
    if config.messaging.webhook.enabled {
        if !config.messaging.webhook.routes.is_empty() {
            router = router.route("/webhooks/hook/:name", post(generic_webhook_route));
            let names: Vec<&str> = config
                .messaging
                .webhook
                .routes
                .iter()
                .map(|r| r.name.as_str())
                .collect();
            tracing::info!(
                "gateway webhook route mounted: /webhooks/hook/:name (routes: {})",
                names.join(", ")
            );
        }
        // Dynamic subscriptions — file hot-reloaded per request (hermes
        // webhook_subscriptions.json). Static platform routes above keep
        // precedence over the wildcard.
        router = router.route("/webhooks/:name", post(dynamic_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/:name (dynamic subscriptions)");
    }
    if config.messaging.bluebubbles.enabled {
        router = router.route("/webhooks/bluebubbles", post(bluebubbles_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/bluebubbles");
    }
    if config.messaging.feishu.enabled {
        router = router.route("/webhooks/feishu", post(feishu_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/feishu");
    }
    if config.messaging.sms.enabled {
        router = router.route("/webhooks/twilio", post(twilio_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/twilio");
    }
    if config.messaging.teams.enabled {
        router = router.route("/webhooks/teams", post(teams_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/teams");
    }
    if config.messaging.line.enabled {
        router = router.route("/webhooks/line", post(line_webhook_route));
        router = router.route("/line/media/:token/:filename", get(line_media_route));
        tracing::info!("gateway webhook route mounted: /webhooks/line (+/line/media)");
    }
    if config.messaging.google_chat.enabled {
        router = router.route("/webhooks/googlechat", post(google_chat_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/googlechat");
    }
    if config.messaging.raft.enabled {
        router = router.route("/webhooks/raft/wake", post(raft_wake_route));
        tracing::info!("gateway webhook route mounted: /webhooks/raft/wake");
    }
    if config.messaging.a2a.enabled {
        router = router
            .route("/a2a", post(a2a_rpc_route))
            .route("/.well-known/agent-card.json", get(a2a_agent_card_route))
            .route("/.well-known/agent.json", get(a2a_agent_card_route));
        tracing::info!("gateway A2A routes mounted: /a2a + /.well-known/agent-card.json");
    }
    router
}

/// Process-wide generic-webhook runtime (rate limits + idempotency),
/// mirroring the hermes WebhookPlatform instance state.
static WEBHOOK_RUNTIME: OnceLock<Arc<crate::webhook_platforms::WebhookRuntime>> = OnceLock::new();

fn webhook_runtime() -> Arc<crate::webhook_platforms::WebhookRuntime> {
    WEBHOOK_RUNTIME
        .get_or_init(|| Arc::new(crate::webhook_platforms::WebhookRuntime::default()))
        .clone()
}

/// Max inbound webhook body (hermes: 1 MiB).
const WEBHOOK_BODY_LIMIT: usize = 1024 * 1024;

async fn generic_webhook_route(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let wh = &config.messaging.webhook;
    let Some(route) = wh.routes.iter().find(|r| r.name == name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unknown webhook route '{name}'") })),
        )
            .into_response();
    };
    process_generic_webhook(state, wh.rate_limit, route, headers, body).await
}

/// Dynamic webhook subscriptions (hermes `webhook_subscriptions.json`):
/// the file is re-read on every request, so `ulnclaw webhook subscribe`
/// takes effect without a gateway restart. Mounted at `/webhooks/:name`
/// (static platform routes keep precedence).
async fn dynamic_webhook_route(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let wh = &config.messaging.webhook;
    let subs = crate::webhook_subscriptions::load_subscriptions();
    let Some(sub) = subs.get(&name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unknown webhook route '{name}'") })),
        )
            .into_response();
    };
    let route = crate::webhook_subscriptions::to_webhook_route(&name, &sub);
    process_generic_webhook(state, wh.rate_limit, route, headers, body).await
}

// ── Webhook subscription management API (desktop Webhooks panel) ─────────

fn subscription_row(name: &str, sub: &crate::webhook_subscriptions::Subscription, base: &str) -> Value {
    let preview: String = if sub.secret.len() > 4 {
        format!("{}…", &sub.secret[..4])
    } else {
        String::new()
    };
    json!({
        "name": name,
        "url": format!("{base}/webhooks/{name}"),
        "description": sub.description,
        "events": sub.events,
        "deliver": sub.deliver,
        "deliver_only": sub.deliver_only,
        "script": sub.script,
        "created_at": sub.created_at,
        "has_secret": !sub.secret.is_empty(),
        "secret_preview": preview,
    })
}

/// `GET /api/webhooks/subscriptions` — list dynamic webhook subscriptions
/// (secrets masked to a 4-char preview; desktop Webhooks panel).
async fn webhook_subscriptions_list(State(state): State<Arc<GatewayState>>) -> Response {
    let base = crate::webhook_subscriptions::base_url(&state.agent.context().config);
    let subs = crate::webhook_subscriptions::load_subscriptions();
    let rows: Vec<Value> = subs
        .iter()
        .map(|(name, sub)| subscription_row(name, sub, &base))
        .collect();
    Json(json!({ "base_url": base, "subscriptions": rows })).into_response()
}

#[derive(Deserialize)]
struct WebhookSubscribeBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    events: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    skills: Option<String>,
    #[serde(default)]
    deliver: Option<String>,
    #[serde(default)]
    deliver_chat_id: Option<String>,
    #[serde(default)]
    deliver_only: bool,
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    secret: Option<String>,
}

/// `POST /api/webhooks/subscriptions` — create or update a subscription
/// (same validation as `ulnclaw webhook subscribe`; hot-reloaded by the
/// gateway on the next request).
async fn webhook_subscriptions_create(
    axum::Json(body): axum::Json<WebhookSubscribeBody>,
) -> Response {
    let opts = crate::webhook_subscriptions::SubscribeOptions {
        description: body.description,
        events: body.events,
        secret: body.secret,
        prompt: body.prompt,
        skills: body.skills,
        deliver: body.deliver,
        deliver_chat_id: body.deliver_chat_id,
        deliver_only: body.deliver_only,
        script: body.script,
    };
    match crate::webhook_subscriptions::cmd_subscribe(&body.name, &opts) {
        Ok(message) => Json(json!({
            "ok": true,
            "name": crate::webhook_subscriptions::normalize_name(&body.name),
            "message": message.trim(),
        }))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

/// `DELETE /api/webhooks/subscriptions/:name` — remove a subscription.
async fn webhook_subscriptions_delete(Path(name): Path<String>) -> Response {
    let normalized = name.trim().to_lowercase();
    let mut subs = crate::webhook_subscriptions::load_subscriptions();
    if subs.remove(&normalized).is_none() {
        return not_found(&format!("no webhook subscription named '{normalized}'"));
    }
    if let Err(e) = crate::webhook_subscriptions::save_subscriptions(&subs) {
        return server_error(&e);
    }
    Json(json!({ "ok": true, "removed": normalized })).into_response()
}

#[derive(Deserialize)]
struct WebhookTestBody {
    #[serde(default)]
    payload: Option<String>,
}

/// `POST /api/webhooks/subscriptions/:name/test` — fire a signed test
/// payload at the subscription's own webhook URL (`ulnclaw webhook test`).
async fn webhook_subscriptions_test(
    Path(name): Path<String>,
    axum::Json(body): axum::Json<WebhookTestBody>,
) -> Response {
    let normalized = name.trim().to_lowercase();
    let payload = body.payload.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::webhook_subscriptions::cmd_test(&normalized, payload.as_deref())
    })
    .await
    .unwrap_or_else(|e| Err(format!("task failed: {e}")));
    match result {
        Ok(message) => Json(json!({ "ok": true, "message": message.trim() })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// Shared generic-webhook pipeline (signature check, rate limit,
/// idempotency, event filter, prompt render, deliver/agent turn) used by
/// both static config routes (`/webhooks/hook/:name`) and dynamic
/// subscriptions (`/webhooks/:name`).
async fn process_generic_webhook(
    state: Arc<GatewayState>,
    rate_limit: u32,
    route: crate::webhook_platforms::WebhookRoute,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    use crate::webhook_platforms as wp;
    let name = route.name.clone();
    let ack = |status: &str| -> Response {
        (StatusCode::OK, Json(json!({ "status": status, "route": name }))).into_response()
    };
    let config = &state.agent.context().config;
    if body.len() > WEBHOOK_BODY_LIMIT {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "webhook body exceeds 1 MiB limit" })),
        )
            .into_response();
    }
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.as_str().to_string(), s.to_string())))
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if !wp::webhook_signature_ok(&route.name, &body, &header_pairs, &route.secret, now as i64) {
        tracing::warn!("[webhook] route '{}' rejected: invalid signature", route.name);
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "invalid signature" })),
        )
            .into_response();
    }
    let runtime = webhook_runtime();
    if wp::webhook_rate_limited(&runtime, &route.name, rate_limit, now).await {
        tracing::warn!("[webhook] route '{}' rate limited", route.name);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "rate limit exceeded" })),
        )
            .into_response();
    }
    // Idempotency: X-Webhook-Delivery-Id wins, then svix-id (hermes order).
    let delivery_id = header_pairs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-webhook-delivery-id"))
        .or_else(|| header_pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case("svix-id")))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    if wp::webhook_already_seen(&runtime, &delivery_id, now).await {
        tracing::info!("[webhook] route '{}' duplicate delivery '{delivery_id}' acked", route.name);
        return ack("duplicate");
    }
    let (allowed, event) = wp::webhook_event_allowed(&route, &header_pairs);
    if !allowed {
        tracing::info!(
            "[webhook] route '{}' skipping event '{event}' (not in filter)",
            route.name
        );
        return ack("skipped");
    }
    let body_text = String::from_utf8_lossy(&body).to_string();
    let prompt = wp::webhook_render_prompt(&route.prompt, &event, &body_text);
    if route.deliver_only {
        wp::webhook_deliver(config, &route, &prompt).await;
        return ack("delivered");
    }
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let event_msg = crate::messaging::MessageEvent {
        platform: "webhook".into(),
        chat_id: route.name.clone(),
        sender_id: "webhook".into(),
        sender_name: format!("webhook:{}", route.name),
        text: prompt,
        message_id: delivery_id.clone(),
        attachments: Vec::new(),
    };
    let reply = match dispatcher.handle_event(event_msg).await {
        Ok(outcome) => outcome.reply,
        Err(e) => {
            tracing::error!("[webhook] route '{}' agent turn failed: {e}", route.name);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("agent turn failed: {e}") })),
            )
                .into_response();
        }
    };
    wp::webhook_deliver(config, &route, &reply).await;
    ack("ok")
}

async fn whatsapp_verify_route(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let cfg = &state.agent.context().config.messaging.whatsapp_cloud;
    let query: Vec<(String, String)> = params.into_iter().collect();
    match crate::webhook_platforms::whatsapp_verify(cfg, &query) {
        Ok(challenge) => (StatusCode::OK, challenge).into_response(),
        Err(_) => StatusCode::FORBIDDEN.into_response(),
    }
}

async fn whatsapp_webhook_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let cfg = &config.messaging.whatsapp_cloud;
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    match crate::webhook_platforms::whatsapp_handle_webhook(
        cfg,
        &dispatcher,
        pairing.as_ref(),
        &body,
        &signature,
    )
    .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => {
            let message = e.to_string();
            tracing::warn!("whatsapp webhook rejected: {message}");
            if message.contains("X-Hub-Signature-256") {
                // Invalid signature: refuse loudly so misconfigurations
                // surface; Meta retries are harmless here.
                StatusCode::FORBIDDEN.into_response()
            } else {
                // Parse/size issues: 200 stops Meta's retry storm (hermes
                // logs and acks).
                StatusCode::OK.into_response()
            }
        }
    }
}

async fn feishu_webhook_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let cfg = &config.messaging.feishu;
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    let result = crate::feishu::feishu_handle_webhook(
        cfg,
        &dispatcher,
        pairing.as_ref(),
        &body,
        &header_pairs,
    )
    .await;
    let status = match result.status {
        200 => StatusCode::OK,
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        413 => StatusCode::PAYLOAD_TOO_LARGE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, axum::Json(result.body)).into_response()
}

async fn twilio_webhook_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let cfg = &config.messaging.sms;
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    let result = crate::sms::sms_handle_webhook(
        cfg,
        &dispatcher,
        pairing.as_ref(),
        &body,
        &header_pairs,
    )
    .await;
    let status = match result.status {
        200 => StatusCode::OK,
        400 => StatusCode::BAD_REQUEST,
        403 => StatusCode::FORBIDDEN,
        413 => StatusCode::PAYLOAD_TOO_LARGE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/xml")],
        result.body,
    )
        .into_response()
}

async fn teams_webhook_route(
    State(state): State<Arc<GatewayState>>,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    let result = crate::teams::teams_handle_webhook(&dispatcher, pairing.as_ref(), &body).await;
    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, axum::Json(result.body)).into_response()
}

async fn line_webhook_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    let result =
        crate::line::line_handle_webhook(&dispatcher, pairing.as_ref(), &body, &header_pairs)
            .await;
    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, axum::Json(result.body)).into_response()
}

/// Serve outbound media registered by the LINE adapter (hermes
/// `_handle_media`): token-gated, TTL-bounded, allowed-roots checked.
async fn line_media_route(Path((token, _filename)): Path<(String, String)>) -> Response {
    match crate::line::line_serve_media(&token).await {
        crate::line::LineMediaResult::Found(bytes, mime) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, mime)],
            bytes,
        )
            .into_response(),
        crate::line::LineMediaResult::NotFound => {
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
        crate::line::LineMediaResult::Gone => (StatusCode::GONE, "gone").into_response(),
        crate::line::LineMediaResult::Forbidden => {
            (StatusCode::FORBIDDEN, "forbidden").into_response()
        }
    }
}

async fn google_chat_webhook_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    let result = crate::google_chat::google_chat_handle_webhook(
        &dispatcher,
        pairing.as_ref(),
        &body,
        &header_pairs,
    )
    .await;
    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, axum::Json(result.body)).into_response()
}

async fn raft_wake_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let result = crate::raft::raft_handle_wake(
        &config.messaging.raft,
        &dispatcher,
        &body,
        &header_pairs,
    )
    .await;
    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, axum::Json(result.body)).into_response()
}

async fn a2a_rpc_route(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let result = crate::a2a::a2a_handle_rpc(&dispatcher, &body, &header_pairs).await;
    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, axum::Json(result.body)).into_response()
}

async fn a2a_agent_card_route() -> Response {
    match crate::a2a::a2a_agent_card_response() {
        Some(card) => (StatusCode::OK, axum::Json(card)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn bluebubbles_webhook_route(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let cfg = &config.messaging.bluebubbles;
    let password_param = params.get("password").cloned();
    let password_header = ["x-password", "x-guid", "x-bluebubbles-guid"]
        .iter()
        .find_map(|name| headers.get(*name).and_then(|v| v.to_str().ok()))
        .map(|s| s.to_string());
    let body_text = String::from_utf8_lossy(&body).to_string();
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    let pairing = if config.messaging.pairing {
        Some(crate::pairing::PairingStore::open(&crate::config::ulnclaw_home()))
    } else {
        None
    };
    match crate::webhook_platforms::bluebubbles_handle_webhook(
        cfg,
        &dispatcher,
        pairing.as_ref(),
        &body_text,
        password_param.as_deref(),
        password_header.as_deref(),
    )
    .await
    {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(e) => {
            let message = e.to_string();
            if message.contains("unauthorized") {
                tracing::warn!("bluebubbles webhook rejected: {message}");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "unauthorized" })),
                )
                    .into_response()
            } else if message.contains("parse") || message.contains("invalid") {
                tracing::warn!("bluebubbles webhook bad payload: {message}");
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": message })),
                )
                    .into_response()
            } else {
                // Ack everything else so the BlueBubbles server does not
                // retry-storm us (hermes logs and returns 200).
                tracing::error!("bluebubbles webhook error: {message}");
                (StatusCode::OK, "ok").into_response()
            }
        }
    }
}

async fn msgraph_webhook_route(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Response {
    let config = &state.agent.context().config;
    let cfg = &config.messaging.msgraph;
    let query: Vec<(String, String)> = params.into_iter().collect();
    // Subscription validation: echo validationToken as text/plain.
    if let Some(token) = crate::webhook_platforms::msgraph_validation_token(&query) {
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            token,
        )
            .into_response();
    }
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    // hermes `_handle_notification` status semantics: 202 when anything
    // was ingested or deduped (Graph acks and stops retrying), 403 when
    // the whole batch failed clientState auth, 400 for malformed /
    // resource-not-accepted batches, 413 oversize.
    match crate::webhook_platforms::msgraph_handle_webhook(cfg, &dispatcher, &body, &query).await
    {
        Ok(outcome) => {
            if outcome.accepted > 0 || outcome.duplicates > 0 {
                StatusCode::ACCEPTED.into_response()
            } else if outcome.auth_rejected > 0 && outcome.other_rejected == 0 {
                StatusCode::FORBIDDEN.into_response()
            } else {
                StatusCode::BAD_REQUEST.into_response()
            }
        }
        Err(e) => {
            tracing::warn!("msgraph webhook rejected: {e}");
            if e.to_string().contains("too large") {
                StatusCode::PAYLOAD_TOO_LARGE.into_response()
            } else {
                StatusCode::BAD_REQUEST.into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Profile multiplexing — `/p/<profile>/...` route mirrors
// ---------------------------------------------------------------------------
//
// Hermes api_server registers every route twice: natively and under
// `/p/{profile}<path>` (gateway multiplexing). With `[gateway]
// multiplex_profiles = true` each mirror is backed by its own agent/store
// built from the `[profiles.<name>]` override and a profile-scoped home
// (`<home>/profiles/<name>`); unknown profiles 404. With multiplexing OFF
// the prefix is accepted but ignored — the default profile serves it
// (hermes `_resolve_request_profile` parity, so a would-be valid route is
// never 404'd just because multiplexing is disabled).

/// Async factory that builds the router for one profile (lazy, cached).
pub type ProfileRouterBuilder = Arc<
    dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Router>> + Send>>
        + Send
        + Sync,
>;

/// Factory for per-profile secret scopes (hermes `_profile_runtime_scope`
/// parity). Builds the profile's `.env` + external-source secret mapping;
/// [`profile_dispatch`] installs it around every `/p/<profile>/...`
/// request so scoped credential reads resolve against the right profile.
pub type ProfileScopeBuilder =
    Arc<dyn Fn(&str) -> std::collections::HashMap<String, String> + Send + Sync>;

/// What a resolved `/p/<profile>` request gets: router + secret scope.
#[derive(Clone)]
struct ResolvedProfile {
    router: Router,
    scope: Option<std::sync::Arc<std::collections::HashMap<String, String>>>,
}

/// Multiplex hub: default router + per-profile router cache + policy.
pub struct ProfileHub {
    /// `[gateway] multiplex_profiles`.
    multiplex: bool,
    /// Profile names this gateway serves (`[profiles]` keys).
    profiles: std::collections::HashSet<String>,
    /// Default-profile router (serves `/p/*` requests while multiplexing is
    /// off, mirroring hermes' ignore-the-prefix behavior).
    default_router: Router,
    /// Lazily-built per-profile routers (+ secret scopes).
    cache: tokio::sync::Mutex<HashMap<String, ResolvedProfile>>,
    builder: ProfileRouterBuilder,
    scope_builder: Option<ProfileScopeBuilder>,
}

impl ProfileHub {
    pub fn new(
        multiplex: bool,
        profiles: std::collections::HashSet<String>,
        default_router: Router,
        builder: ProfileRouterBuilder,
        scope_builder: Option<ProfileScopeBuilder>,
    ) -> Arc<Self> {
        Arc::new(Self {
            multiplex,
            profiles,
            default_router,
            cache: tokio::sync::Mutex::new(HashMap::new()),
            builder,
            scope_builder,
        })
    }

    /// Whether `[gateway] multiplex_profiles` is on.
    pub fn multiplex_enabled(&self) -> bool {
        self.multiplex
    }

    /// Resolve the router (and secret scope) for a `/p/<profile>` request.
    /// `None` = unknown profile while multiplexing is on (caller 404s).
    async fn resolve(&self, profile: &str) -> Option<ResolvedProfile> {
        if !self.multiplex {
            return Some(ResolvedProfile {
                router: self.default_router.clone(),
                scope: None,
            });
        }
        if !self.profiles.contains(profile) {
            return None;
        }
        {
            let cache = self.cache.lock().await;
            if let Some(resolved) = cache.get(profile) {
                return Some(resolved.clone());
            }
        }
        let built = (self.builder)(profile.to_string()).await.ok()?;
        let scope = self
            .scope_builder
            .as_ref()
            .map(|build| std::sync::Arc::new(build(profile)));
        let resolved = ResolvedProfile {
            router: built,
            scope,
        };
        let mut cache = self.cache.lock().await;
        cache
            .entry(profile.to_string())
            .or_insert_with(|| resolved.clone());
        Some(resolved)
    }
}

/// Dispatch handler for `/p/:profile/*rest` — validates the profile,
/// strips the prefix, and re-dispatches to the profile's router (same
/// handlers, same bearer auth; hermes profile-prefix middleware parity).
async fn profile_dispatch(
    State(hub): State<Arc<ProfileHub>>,
    Path((profile, rest)): Path<(String, String)>,
    mut request: axum::extract::Request,
) -> Response {
    let Some(resolved) = hub.resolve(&profile).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Unknown or unconfigured profile"})),
        )
            .into_response();
    };
    // Rewrite the URI: strip `/p/<profile>` keeping the query string.
    let path = format!("/{}", rest.trim_start_matches('/'));
    let query = request.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let new_uri = format!("{}{}", path, query);
    *request.uri_mut() = match new_uri.parse() {
        Ok(uri) => uri,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": "bad profile path"})))
                .into_response()
        }
    };
    // Install the profile's secret scope around the dispatched request
    // (hermes `_profile_runtime_scope` parity): scoped credential reads
    // inside the request resolve against this profile's `.env`, never
    // against another profile's process-env residue.
    let dispatch = tower::ServiceExt::oneshot(resolved.router, request);
    let outcome = match resolved.scope {
        Some(scope) => crate::secret_scope::scope_secrets(scope, dispatch).await,
        None => dispatch.await,
    };
    match outcome {
        Ok(response) => response,
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "profile dispatch failed"})),
        )
            .into_response(),
    }
}

/// Serve the gateway until interrupted.
pub async fn serve(state: Arc<GatewayState>, host: &str, port: u16) -> Result<()> {
    serve_multiplex(state, None, host, port).await
}

/// Serve with optional `/p/<profile>` multiplexing.
pub async fn serve_multiplex(
    state: Arc<GatewayState>,
    hub: Option<Arc<ProfileHub>>,
    host: &str,
    port: u16,
) -> Result<()> {
    // Desktop UI bridge (P231): when the gateway is a desktop shell's
    // child (ULNCLAW_DESKTOP=1), expose desktop-tool events over SSE
    // and answer read_terminal via an HTTP round-trip.
    crate::desktop_bridge::install();
    // Fail-closed secret resolution while multiplexing (hermes
    // `set_multiplex_active` at gateway startup): with multiplexing on,
    // any credential read outside a profile scope errors loudly instead
    // of leaking another profile's process-env value.
    crate::secret_scope::set_multiplex_active(
        hub.as_ref().map(|hub| hub.multiplex_enabled()).unwrap_or(false),
    );
    let mut app = router(state.clone());
    if let Some(hub) = hub {
        // Register the mirror for ALL methods the native table uses.
        let mirror = Router::new()
            .route(
                "/p/:profile/*rest",
                get(profile_dispatch)
                    .post(profile_dispatch)
                    .put(profile_dispatch)
                    .delete(profile_dispatch)
                    .patch(profile_dispatch)
                    .head(profile_dispatch)
                    .options(profile_dispatch),
            )
            .with_state(hub.clone());
        app = app.merge(mirror);
        tracing::info!(
            "gateway profile multiplexing: {} ({} profile(s) configured)",
            if hub.multiplex { "on" } else { "off (prefix ignored)" },
            hub.profiles.len()
        );
    }
    let app = with_cors(app);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| AgentError::config(format!("gateway bind {}: {}", addr, e)))?;
    tracing::info!(
        "ulnclaw gateway listening on http://{} (auth: {})",
        addr,
        if state.key.is_some() { "bearer token" } else { "none" }
    );
    println!(
        "ulnclaw gateway listening on http://{} (auth: {})",
        addr,
        if state.key.is_some() { "bearer token" } else { "none" }
    );
    // Raft bridge spawn (hermes `_spawn_bridge`) — the wake endpoint
    // rides this listener, so the bridge gets its URL from the bound
    // address.
    let raft_bridge = if state.agent.context().config.messaging.raft.enabled {
        let endpoint_host = if host == "0.0.0.0" || host == "::" || host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            host.to_string()
        };
        let endpoint = format!("http://{endpoint_host}:{port}/webhooks/raft/wake");
        crate::raft::spawn_bridge(&state.agent.context().config.messaging.raft, &endpoint)
    } else {
        None
    };
    let serve_result = axum::serve(listener, app)
        .await
        .map_err(|e| AgentError::config(format!("gateway serve: {}", e)));
    if let Some(handle) = raft_bridge {
        crate::raft::stop_bridge(handle).await;
    }
    serve_result
}

// ---------------------------------------------------------------------------
// CORS middleware
// ---------------------------------------------------------------------------

/// Wrap a router in the local-app CORS layer.
///
/// Local-app CORS (desktop GUI / browser dashboards): the gateway binds
/// 127.0.0.1 by default and is additionally key-gated, so permissive
/// CORS for local origins matches the hermes dashboard model.
pub fn with_cors(app: Router) -> Router {
    app.layer(axum::middleware::from_fn(cors_middleware))
}

/// Permissive CORS for local apps (desktop GUI, browser dashboards).
/// Echoes the request Origin when present; handles preflight OPTIONS.
async fn cors_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let origin = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("*")
        .to_string();
    let is_preflight = request.method() == axum::http::Method::OPTIONS;
    let mut response = if is_preflight {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = axum::http::StatusCode::NO_CONTENT;
        response
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        origin.parse().unwrap_or_else(|_| "*".parse().unwrap()),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, PUT, PATCH, DELETE, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type, Authorization".parse().unwrap(),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_MAX_AGE,
        "86400".parse().unwrap(),
    );
    response
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The Chronos fire webhook is public like hermes' `PUBLIC_API_PATHS`
/// entry for `/api/cron/fire`: the NAS-minted JWT is the gate, not the
/// dashboard bearer key. Matches the bare path and `/p/<profile>`
/// multiplex mirrors.
fn is_cron_fire_path(path: &str) -> bool {
    if path == "/api/jobs/fire" {
        return true;
    }
    path.strip_prefix("/p/")
        .and_then(|rest| rest.split_once('/'))
        .map(|(_, tail)| tail == "api/jobs/fire")
        .unwrap_or(false)
}

/// The MCP OAuth browser redirect target — open (no bearer key), like
/// hermes' `/api/mcp/oauth/callback/{server}` route. Matches the bare
/// path and `/p/<profile>` multiplex mirrors.
fn is_mcp_oauth_callback_path(path: &str) -> bool {
    if path.starts_with("/api/mcp/oauth/callback/") {
        return true;
    }
    path.strip_prefix("/p/")
        .and_then(|rest| rest.split_once('/'))
        .map(|(_, tail)| tail.starts_with("api/mcp/oauth/callback/"))
        .unwrap_or(false)
}

async fn auth_middleware(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    // Health probes are always open (hermes behavior). Platform webhook
    // ingress is likewise open — Meta/Graph authenticate via HMAC /
    // clientState, not the gateway bearer key.
    if path == "/health"
        || path == "/health/detailed"
        || path == "/v1/health"
        || path.starts_with("/webhooks/")
        || is_cron_fire_path(&path)
        || is_mcp_oauth_callback_path(&path)
    {
        return next.run(request).await;
    }
    let Some(expected) = state.key.as_deref() else {
        return next.run(request).await;
    };
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let token = v.trim().strip_prefix("Bearer ").unwrap_or(v.trim());
            constant_time_eq(token, expected)
        })
        .unwrap_or(false);
    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"message": "invalid or missing bearer token", "type": "auth_error"}})),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// Health / discovery
// ---------------------------------------------------------------------------

async fn health(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "ulnclaw-gateway",
        "version": crate::VERSION,
        "model": state.model_name,
    }))
}

async fn health_detailed(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let sessions = state.store.list_session_rows(1).map(|rows| rows.len()).unwrap_or(0);
    let runs = state.runs.lock().await.len();
    Json(json!({
        "status": "ok",
        "service": "ulnclaw-gateway",
        "version": crate::VERSION,
        "model": state.model_name,
        "provider": state.provider_name,
        "auth_required": state.key.is_some(),
        "sessions_total_at_least": sessions,
        "runs_tracked": runs,
    }))
}

async fn models(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": [{
            "id": state.model_name,
            "object": "model",
            "created": now_secs() as u64,
            "owned_by": "ulnclaw",
        }],
    }))
}

#[derive(serde::Deserialize)]
struct ModelOptionsQuery {
    refresh: Option<String>,
    include_unconfigured: Option<String>,
    explicit_only: Option<String>,
}

fn query_flag(value: Option<&String>) -> bool {
    value
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .unwrap_or(false)
}

/// `GET /api/model/options` — provider/model inventory for pickers
/// (hermes `_handle_model_options` via `build_model_options_payload`).
/// Rows: the configured current provider (models.dev catalog enrichment),
/// `[providers.<slug>]` config entries, env-authenticated canonical
/// providers, and — with `include_unconfigured` — skeleton rows with
/// setup hints. `?refresh=true` busts the catalog cache and probes every
/// configured OpenAI-compatible endpoint; `?explicit_only=true` keeps
/// only explicitly-configured rows (hermes query params).
async fn model_options(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ModelOptionsQuery>,
) -> Json<Value> {
    let opts = crate::model_inventory::InventoryOptions {
        refresh: query_flag(query.refresh.as_ref()),
        include_unconfigured: query_flag(query.include_unconfigured.as_ref()),
        explicit_only: query_flag(query.explicit_only.as_ref()),
    };
    let provider = state.provider_name.clone();
    let model = state.model_name.clone();
    let payload = tokio::task::spawn_blocking(move || {
        let cfg = crate::config::UlncLawConfig::load(None).unwrap_or_default();
        let mut input = crate::model_inventory::InventoryInput::from_config(&cfg);
        input.current_provider = provider;
        input.current_model = model;
        crate::model_inventory::build_model_options_payload(&input, &opts)
    })
    .await
    .unwrap_or_else(|_| json!({"providers": [], "model": "", "provider": ""}));
    Json(payload)
}

/// The model a session is locked to, when it differs from the gateway's
/// configured model (session rows stamp the configured model at creation).
fn session_model_override(state: &GatewayState, session_id: &str) -> Option<String> {
    let row = state.store.get_session_row(session_id).ok().flatten()?;
    let model = row.model.filter(|m| !m.is_empty())?;
    if model == state.model_name {
        None
    } else {
        Some(model)
    }
}

/// `POST /api/sessions/:id/model` — acknowledge + persist a session model
/// lock (hermes `_handle_session_model_lock`). The lock is enforced on
/// every subsequent turn of this session via a per-task model override.
async fn lock_session_model(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match state.store.get_session_row(&id) {
        Ok(None) => return not_found(&format!("session {} not found", id)),
        Err(e) => return server_error(&e.to_string()),
        Ok(Some(_)) => {}
    }
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(model) = model else {
        return bad_request("model is required", Some("model_required"));
    };
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| state.provider_name.clone());
    if let Err(e) = state.store.set_session_model(&id, &model) {
        return server_error(&e.to_string());
    }
    Json(json!({
        "object": "ulnclaw.session.model_lock",
        "session_id": id,
        "runtime": {
            "provider": provider,
            "model": model,
            "route_source": "api_request",
            "model_lock": "accepted",
        },
    }))
    .into_response()
}

/// `GET /api/sessions/:id/recap` — instant local activity recap (hermes
/// `build_recap`, shared with the CLI `/recap`). No LLM call.
async fn session_recap(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let row = match state.store.get_session_row(&id) {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&format!("session {} not found", id)),
        Err(e) => return server_error(&e.to_string()),
    };
    let messages = match state.store.load_messages(&id) {
        Ok(messages) => messages,
        Err(e) => return server_error(&e.to_string()),
    };
    let recap = crate::session::recap::build_recap(&messages, row.title.as_deref(), Some(&row.id));
    Json(json!({
        "object": "ulnclaw.session.recap",
        "session_id": row.id,
        "recap": recap,
    }))
    .into_response()
}

/// Await an agent future with the session's model lock (if any) installed
/// as a per-task override.
async fn await_with_model_override<F, T>(override_model: Option<String>, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match override_model {
        Some(model) => crate::agent::model_override_scope(model, future).await,
        None => future.await,
    }
}

/// `GET /v1/delegations` — background delegation registry (ulnclaw ops
/// extension over hermes async_delegation).
async fn list_delegations_http(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let snapshot = crate::async_delegation::list_delegations();
    let known_ids: std::collections::HashSet<String> =
        snapshot.iter().map(|r| r.id.clone()).collect();
    let mut records: Vec<Value> = snapshot
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "status": r.status,
                "tasks": r.tasks,
                "parent_session_key": r.parent_session_key,
                "created_ms": r.created_ms,
                "finished_ms": r.finished_ms,
                "log_dir": r.log_dir.display().to_string(),
            })
        })
        .collect();
    // Cross-restart history from the durable registry (in-memory wins on
    // id collisions; DB rows cover delegations from previous processes).
    let home = state.agent.context().home.clone();
    for (id, origin, row_state, dispatched_at, completed_at, _result_json, delivery_attempts) in
        state.store.delegation_rows(200)
    {
        if known_ids.contains(&id) {
            continue;
        }
        records.push(json!({
            "id": id,
            "status": row_state,
            "parent_session_key": origin,
            "created_ms": (dispatched_at * 1000.0) as i64,
            "finished_ms": completed_at.map(|s| (s * 1000.0) as i64),
            "log_dir": crate::async_delegation::live_root(&home)
                .join(&id)
                .display()
                .to_string(),
            "delivery_attempts": delivery_attempts,
            "persisted": true,
        }));
    }
    Json(json!({"delegations": records}))
}

/// `GET /v1/delegations/:id` — one delegation record + consolidated result
/// once finished (ulnclaw ops extension).
async fn get_delegation_http(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(record) = crate::async_delegation::get_delegation(&id) {
        let home = state.agent.context().home.clone();
        let result = crate::async_delegation::read_result(&home, &id);
        return Json(json!({
            "id": record.id,
            "status": record.status,
            "tasks": record.tasks,
            "parent_session_key": record.parent_session_key,
            "created_ms": record.created_ms,
            "finished_ms": record.finished_ms,
            "log_dir": record.log_dir.display().to_string(),
        "result": result,
    }))
    .into_response();
    }
    // Durable-registry fallback: delegations from previous processes.
    if let Some((_, origin, row_state, dispatched_at, completed_at, result_json, delivery_attempts)) = state
        .store
        .delegation_rows(500)
        .into_iter()
        .find(|(row_id, _, _, _, _, _, _)| row_id == &id)
    {
        let home = state.agent.context().home.clone();
        let result = crate::async_delegation::read_result(&home, &id)
            .or_else(|| result_json.and_then(|raw| serde_json::from_str(&raw).ok()));
        return Json(json!({
            "id": id,
            "status": row_state,
            "parent_session_key": origin,
            "created_ms": (dispatched_at * 1000.0) as i64,
            "finished_ms": completed_at.map(|s| (s * 1000.0) as i64),
            "log_dir": crate::async_delegation::live_root(&home)
                .join(&id)
                .display()
                .to_string(),
            "result": result,
            "delivery_attempts": delivery_attempts,
            "persisted": true,
        }))
        .into_response();
    }
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": {"message": format!("delegation '{id}' not found"), "type": "invalid_request_error"}})),
    )
        .into_response()
}

/// `GET /api/desktop/events` — SSE stream of desktop UI bridge events
/// (P231). Every envelope is `{session_id, event, payload}`; the webview
/// routes `terminal.close` / `pane.reveal` / `preview.open` /
/// `message.reaction` to its panes and answers `terminal.read` requests
/// via `POST /api/desktop/read-response`.
async fn desktop_events() -> axum::response::Sse<impl futures::Stream<Item = std::result::Result<axum::response::sse::Event, std::convert::Infallible>>> {
    let rx = crate::desktop_bridge::subscribe();
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    let data = json!({
                        "session_id": envelope.session_id,
                        "event": envelope.event,
                        "payload": envelope.payload,
                    });
                    let event = axum::response::sse::Event::default()
                        .json_data(data)
                        .ok()?;
                    return Some((Ok(event), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    axum::response::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new())
}

/// `POST /api/desktop/read-response` — the webview's answer to a pending
/// `terminal.read` request (P231). Body: `{id, ok, result}`.
async fn desktop_read_response(Json(body): Json<Value>) -> Response {
    let Some(id) = body.get("id").and_then(|v| v.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "id is required"})),
        )
            .into_response();
    };
    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    let result = body
        .get("result")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let resolved = crate::desktop_bridge::resolve_read(
        id,
        if ok { Ok(result) } else { Err(result) },
    );
    if resolved {
        Json(json!({"resolved": true})).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("no pending terminal read with id '{id}'")})),
        )
            .into_response()
    }
}

/// `GET /v1/browser/status` — current browser CDP endpoint configuration
/// (ulnclaw ops extension over hermes `/browser` UX).
async fn browser_status(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let _ = state;
    let managed_running = crate::browser::managed_running().await;
    if crate::browser::camofox::is_camofox_mode() {
        let available = crate::browser::camofox::check_available().await;
        let vnc = crate::browser::camofox::vnc_url().await;
        return Json(json!({
            "configured": true,
            "backend": "camofox",
            "source": "env",
            "endpoint": crate::browser::camofox::camofox_url(),
            "mode": "camofox",
            "available": available,
            "vnc_url": vnc,
            "managed_running": false,
        }));
    }
    match crate::browser::endpoint_with_source() {
        Some((source, raw)) => {
            let mode = if crate::browser::is_auto_mode(&raw) {
                "managed"
            } else {
                "endpoint"
            };
            Json(json!({
                "configured": true,
                "backend": "cdp",
                "source": source,
                "endpoint": raw,
                "mode": mode,
                "managed_running": managed_running,
            }))
        }
        None => Json(json!({
            "configured": false,
            "backend": null,
            "source": null,
            "endpoint": null,
            "mode": "none",
            "managed_running": managed_running,
        })),
    }
}

#[derive(Deserialize)]
struct BrowserConnectRequest {
    #[serde(default)]
    url: String,
}

/// `POST /v1/browser/connect` — point the browser tools at a CDP endpoint
/// for the process lifetime (hermes `/browser connect`, which live-sets
/// `BROWSER_CDP_URL`). Accepts ws://, wss://, http(s):// discovery bases,
/// or `auto` (managed local browser). The endpoint is verified before the
/// override sticks.
async fn browser_connect(Json(request): Json<BrowserConnectRequest>) -> Response {
    let url = request.url.trim().to_string();
    if url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "url is required", "type": "invalid_request_error"}})),
        )
            .into_response();
    }
    if let Err(e) = crate::browser::set_cdp_override(&url) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": e.to_string(), "type": "invalid_request_error"}})),
        )
            .into_response();
    }
    // Verify reachability before the override sticks (auto mode defers the
    // managed launch to first use).
    if !crate::browser::is_auto_mode(&url) {
        let reachable = match crate::browser::resolve_endpoint(&url) {
            Ok(endpoint) => {
                if let Some(http_base) = &endpoint.http_base {
                    crate::browser::discover_browser_ws(http_base).await.is_ok()
                } else if let Some(ws) = &endpoint.browser_ws {
                    crate::browser::CdpClient::connect(ws).await.is_ok()
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        if !reachable {
            crate::browser::clear_cdp_override();
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {
                    "message": format!("browser endpoint '{url}' is unreachable (no DevTools discovery response)"),
                    "type": "invalid_request_error"
                }})),
            )
                .into_response();
        }
    }
    Json(json!({"connected": true, "endpoint": url})).into_response()
}

/// `POST /v1/browser/disconnect` — clear the live CDP override (hermes
/// `/browser disconnect`); the tools fall back to `ULNCLAW_BROWSER_CDP`.
async fn browser_disconnect() -> Json<Value> {
    crate::browser::clear_cdp_override();
    Json(json!({"disconnected": true}))
}

async fn capabilities(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    Json(json!({
        "service": "ulnclaw-gateway",
        "version": crate::VERSION,
        "model": state.model_name,
        "session_header": "X-Ulnclaw-Session-Id",
        "endpoints": {
            "chat_completions": true,
            "responses": true,
            "runs": true,
            "runs_events_sse": true,
            "run_approval": true,
            "sessions": true,
            "session_patch": true,
            "session_fork": true,
            "jobs": true,
            "skills": true,
            "toolsets": true,
            "model_options": true,
            "session_model_lock": true,
            "session_recap": true,
            "delegations": true,
            "browser": true,
            "metrics": true,
            "usage": true,
            "streaming": true,
        },
    }))
}

// ---------------------------------------------------------------------------
// /v1/chat/completions
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChatCompletionRequest {
    #[serde(default)]
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: Option<Value>,
}

fn message_text(content: &Option<Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(|v| v.as_str()) {
                Some("text") => part.get("text").and_then(|v| v.as_str()).map(String::from),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(SESSION_HEADER)
        .or_else(|| headers.get("x-hermes-session-id"))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

async fn chat_completions(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Json(request): Json<ChatCompletionRequest>,
) -> Response {
    let session_id = session_id_from_headers(&headers);

    // History: resumed session rows, or the request's own messages.
    let (history, prompt) = if let Some(ref sid) = session_id {
        // Stored system prompts are stale duplicates — the agent prepends a
        // fresh one each run.
        let stored = state
            .store
            .load_messages(sid)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role != Role::System)
            .collect::<Vec<_>>();
        let prompt = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| message_text(&m.content))
            .unwrap_or_default();
        (stored, prompt)
    } else {
        let mut history = Vec::new();
        let mut prompt = String::new();
        for message in &request.messages {
            let text = message_text(&message.content);
            match message.role.as_str() {
                "user" => {
                    if !prompt.is_empty() {
                        history.push(Message {
                            role: Role::User,
                            content: Some(std::mem::take(&mut prompt)),
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                        });
                    }
                    prompt = text;
                }
                "assistant" => history.push(Message {
                    role: Role::Assistant,
                    content: Some(text),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                }),
                "tool" => history.push(Message {
                    role: Role::Tool,
                    content: Some(text),
                    tool_calls: None,
                    tool_call_id: message.content.as_ref().and_then(|c| c.get("tool_call_id")).and_then(|v| v.as_str()).map(String::from),
                    name: None,
                }),
                _ => {}
            }
        }
        (history, prompt)
    };

    if prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "no user message found", "type": "invalid_request_error"}})),
        )
            .into_response();
    }

    state
        .metrics
        .chat_completions
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let history_arg = if history.is_empty() { None } else { Some(history) };
    if request.stream {
        return stream_agent_response(state, prompt, history_arg, session_id);
    }
    let override_model = session_id
        .as_deref()
        .and_then(|sid| session_model_override(&state, sid));
    let outcome = await_with_model_override(
        override_model,
        state.agent.run_with_session(&prompt, history_arg, session_id.as_deref()),
    )
    .await;
    match outcome {
        Ok(result) => {
            let mut response = Json(json!({
                "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                "object": "chat.completion",
                "created": now_secs() as u64,
                "model": state.model_name,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": result.content},
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": result.usage.prompt_tokens,
                    "completion_tokens": result.usage.completion_tokens,
                    "total_tokens": result.usage.prompt_tokens + result.usage.completion_tokens,
                },
                "session_id": result.session_id,
            }))
            .into_response();
            state.metrics.record_run(&result.usage, result.tool_calls.len());
            if let Some(ref sid) = result.session_id {
                if let Ok(value) = sid.parse() {
                    response.headers_mut().insert(SESSION_HEADER, value);
                }
            }
            response
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"message": e.to_string(), "type": "agent_error"}})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Streaming (SSE chat.completion.chunk) — hermes stream_delta_callback port
// ---------------------------------------------------------------------------

/// Join handle guard: aborts the agent task when the SSE body is dropped
/// (client disconnect), mirroring hermes' mid-stream interrupt.
struct AbortGuard {
    handle: Option<tokio::task::JoinHandle<Result<crate::agent::RunResult>>>,
}

impl Drop for AbortGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

struct SseState {
    rx: tokio::sync::mpsc::UnboundedReceiver<crate::agent::StreamEvent>,
    guard: AbortGuard,
    completion_id: String,
    created: u64,
    model: String,
    started: bool,
    pending: std::collections::VecDeque<axum::response::sse::Event>,
    finished: bool,
}

/// Stream an agent run as OpenAI-compatible SSE chunks.
///
/// Emits a role chunk, then `delta.content` chunks as the model produces
/// tokens, `hermes.tool.progress` events for tool lifecycle, and a final
/// chunk with `finish_reason` + usage followed by `[DONE]`.
fn stream_agent_response(
    state: Arc<GatewayState>,
    prompt: String,
    history: Option<Vec<Message>>,
    session_id: Option<String>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<crate::agent::StreamEvent>();
    let emitter: Arc<dyn Fn(crate::agent::StreamEvent) + Send + Sync> =
        Arc::new(move |event| {
            let _ = tx.send(event);
        });

    let runner = state.agent.clone();
    let run_session_id = session_id.clone();
    let override_model = session_id
        .as_deref()
        .and_then(|sid| session_model_override(&state, sid));
    let task = tokio::spawn(crate::agent::stream_scope(
        emitter,
        async move {
            await_with_model_override(
                override_model,
                runner.run_with_session(&prompt, history, run_session_id.as_deref()),
            )
            .await
        },
    ));

    let sse_state = SseState {
        rx,
        guard: AbortGuard { handle: Some(task) },
        completion_id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        created: now_secs() as u64,
        model: state.model_name.clone(),
        started: false,
        pending: std::collections::VecDeque::new(),
        finished: false,
    };

    let stream = futures::stream::unfold(sse_state, |mut st| async move {
        use axum::response::sse::Event;
        let make_chunk = |st: &SseState, delta: Value, finish: Value| {
            json!({
                "id": st.completion_id,
                "object": "chat.completion.chunk",
                "created": st.created,
                "model": st.model,
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
            })
        };
        loop {
            if !st.started {
                st.started = true;
                let chunk = make_chunk(&st, json!({"role": "assistant"}), Value::Null);
                return Some((Ok::<_, std::convert::Infallible>(Event::default().json_data(chunk).unwrap()), st));
            }
            if let Some(event) = st.pending.pop_front() {
                return Some((Ok(event), st));
            }
            if st.finished {
                return None;
            }
            match st.rx.recv().await {
                Some(crate::agent::StreamEvent::Delta(delta)) => {
                    let chunk = make_chunk(&st, json!({"content": delta}), Value::Null);
                    return Some((Ok(Event::default().json_data(chunk).unwrap()), st));
                }
                Some(crate::agent::StreamEvent::ToolProgress { tool, status }) => {
                    let payload = json!({"tool": tool, "status": status});
                    let event = Event::default()
                        .event("hermes.tool.progress")
                        .json_data(payload)
                        .unwrap();
                    return Some((Ok(event), st));
                }
                Some(crate::agent::StreamEvent::ToolStarted {
                    name,
                    call_id,
                    arguments,
                }) => {
                    // Tool-call cards for rich clients (hermes desktop
                    // toolsets); plain chat-completions clients ignore
                    // named events they don't know.
                    let payload = json!({
                        "name": name,
                        "call_id": call_id,
                        "arguments": truncate_for_ui(&arguments, 4000),
                    });
                    let event = Event::default()
                        .event("hermes.tool.started")
                        .json_data(payload)
                        .unwrap();
                    return Some((Ok(event), st));
                }
                Some(crate::agent::StreamEvent::ToolCompleted { call_id, result }) => {
                    let payload = json!({
                        "call_id": call_id,
                        "result": truncate_for_ui(&result, 8000),
                    });
                    let event = Event::default()
                        .event("hermes.tool.completed")
                        .json_data(payload)
                        .unwrap();
                    return Some((Ok(event), st));
                }
                None => {
                    // Emitter dropped → the agent task finished. Await it and
                    // queue the terminal events: [error delta] + final chunk
                    // + [DONE].
                    let outcome = match st.guard.handle.take() {
                        Some(handle) => handle.await,
                        None => Ok(Err(AgentError::provider("agent task vanished"))),
                    };
                    let (finish_reason, usage, error_text) = match outcome {
                        Ok(Ok(result)) => (
                            "stop",
                            json!({
                                "prompt_tokens": result.usage.prompt_tokens,
                                "completion_tokens": result.usage.completion_tokens,
                                "total_tokens": result.usage.total_tokens,
                            }),
                            None,
                        ),
                        Ok(Err(e)) => (
                            "error",
                            json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}),
                            Some(e.to_string()),
                        ),
                        Err(e) => (
                            "error",
                            json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}),
                            Some(format!("agent task aborted: {}", e)),
                        ),
                    };
                    if let Some(text) = error_text {
                        let chunk = make_chunk(
                            &st,
                            json!({"content": format!("[error] {}", text)}),
                            Value::Null,
                        );
                        st.pending
                            .push_back(Event::default().json_data(chunk).unwrap());
                    }
                    let final_chunk = json!({
                        "id": st.completion_id,
                        "object": "chat.completion.chunk",
                        "created": st.created,
                        "model": st.model,
                        "choices": [{"index": 0, "delta": {}, "finish_reason": finish_reason}],
                        "usage": usage,
                    });
                    st.pending
                        .push_back(Event::default().json_data(final_chunk).unwrap());
                    st.pending.push_back(Event::default().data("[DONE]"));
                    st.finished = true;
                }
            }
        }
    });

    let sse = axum::response::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    );
    let mut response = sse.into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    if let Some(ref sid) = session_id {
        if let Ok(value) = sid.parse() {
            response.headers_mut().insert(SESSION_HEADER, value);
        }
    }
    response
}

// ---------------------------------------------------------------------------
// /v1/responses — OpenAI Responses API (stateful via previous_response_id)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ResponsesRequest {
    input: Value,
    #[serde(default)]
    previous_response_id: Option<String>,
    #[serde(default)]
    stream: bool,
}

fn input_to_messages(input: &Value) -> Vec<ChatMessage> {
    match input {
        Value::String(text) => vec![ChatMessage {
            role: "user".to_string(),
            content: Some(Value::String(text.clone())),
        }],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                Some(ChatMessage {
                    role: item.get("role")?.as_str()?.to_string(),
                    content: item.get("content").cloned(),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Pending `function_call` item awaiting its completion event.
struct PendingFnCall {
    call_id: String,
    name: String,
    arguments: String,
    item_id: String,
    output_index: u64,
}

struct ResponsesSseState {
    rx: tokio::sync::mpsc::UnboundedReceiver<crate::agent::StreamEvent>,
    guard: AbortGuard,
    gateway: Arc<GatewayState>,
    response_id: String,
    created_at: u64,
    model: String,
    prev_response_id: Option<String>,
    started: bool,
    pending_events: std::collections::VecDeque<axum::response::sse::Event>,
    finished: bool,
    seq: u64,
    output_index: u64,
    message_item_id: String,
    message_open: bool,
    text_parts: Vec<String>,
    emitted_items: Vec<Value>,
    pending_tool_calls: Vec<PendingFnCall>,
}

impl ResponsesSseState {
    fn envelope(&self, status: &str) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
            "status": status,
            "created_at": self.created_at,
            "model": self.model,
        })
    }

    /// Queue a typed Responses SSE event with a monotonic sequence_number.
    fn queue_event(&mut self, event_type: &str, mut data: Value) {
        data["sequence_number"] = json!(self.seq);
        self.seq += 1;
        if let Ok(event) = axum::response::sse::Event::default()
            .event(event_type)
            .json_data(data)
        {
            self.pending_events.push_back(event);
        }
    }

    fn open_message_item(&mut self) {
        if self.message_open {
            return;
        }
        self.message_open = true;
        let item = json!({
            "id": self.message_item_id,
            "type": "message",
            "status": "in_progress",
            "role": "assistant",
            "content": [],
        });
        self.queue_event(
            "response.output_item.added",
            json!({"type": "response.output_item.added", "output_index": self.output_index, "item": item}),
        );
    }
}

/// Stream a `/v1/responses` run as OpenAI Responses SSE events (hermes
/// `_write_responses_sse_stream` port): `response.created`,
/// `response.output_text.delta`, `response.output_item.added/done` for
/// `function_call` + `function_call_output` items, and a terminal
/// `response.completed` / `response.failed` carrying the full envelope.
fn stream_responses_response(
    state: Arc<GatewayState>,
    prompt: String,
    history: Option<Vec<Message>>,
    session_id: Option<String>,
    prev_response_id: Option<String>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<crate::agent::StreamEvent>();
    let emitter: Arc<dyn Fn(crate::agent::StreamEvent) + Send + Sync> =
        Arc::new(move |event| {
            let _ = tx.send(event);
        });

    let runner = state.agent.clone();
    let run_session_id = session_id.clone();
    let override_model = session_id
        .as_deref()
        .and_then(|sid| session_model_override(&state, sid));
    let task = tokio::spawn(crate::agent::stream_scope(
        emitter,
        async move {
            await_with_model_override(
                override_model,
                runner.run_with_session(&prompt, history, run_session_id.as_deref()),
            )
            .await
        },
    ));

    let gateway_state = state.clone();
    let sse_state = ResponsesSseState {
        rx,
        guard: AbortGuard { handle: Some(task) },
        gateway: gateway_state,
        response_id: format!("resp_{}", uuid::Uuid::new_v4()),
        created_at: now_secs() as u64,
        model: state.model_name.clone(),
        prev_response_id,
        started: false,
        pending_events: std::collections::VecDeque::new(),
        finished: false,
        seq: 0,
        output_index: 0,
        message_item_id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
        message_open: false,
        text_parts: Vec::new(),
        emitted_items: Vec::new(),
        pending_tool_calls: Vec::new(),
    };

    let stream = futures::stream::unfold(sse_state, |mut st| async move {
        loop {
            if !st.started {
                st.started = true;
                let mut env = st.envelope("in_progress");
                env["output"] = json!([]);
                st.queue_event(
                    "response.created",
                    json!({"type": "response.created", "response": env}),
                );
            }
            if let Some(event) = st.pending_events.pop_front() {
                return Some((Ok::<_, std::convert::Infallible>(event), st));
            }
            if st.finished {
                return None;
            }
            match st.rx.recv().await {
                Some(crate::agent::StreamEvent::Delta(delta)) => {
                    let message_output_index = st.output_index;
                    st.open_message_item();
                    if !st.message_open {
                        continue;
                    }
                    st.text_parts.push(delta.clone());
                    st.queue_event(
                        "response.output_text.delta",
                        json!({
                            "type": "response.output_text.delta",
                            "item_id": st.message_item_id,
                            "output_index": message_output_index,
                            "content_index": 0,
                            "delta": delta,
                            "logprobs": [],
                        }),
                    );
                }
                Some(crate::agent::StreamEvent::ToolProgress { .. }) => {
                    // Responses clients get structured items instead.
                }
                Some(crate::agent::StreamEvent::ToolStarted { name, call_id, arguments }) => {
                    let item_id = format!("fc_{}", uuid::Uuid::new_v4().simple());
                    let idx = st.output_index;
                    st.output_index += 1;
                    let item = json!({
                        "id": item_id,
                        "type": "function_call",
                        "status": "in_progress",
                        "name": name,
                        "call_id": call_id,
                        "arguments": arguments,
                    });
                    st.emitted_items.push(json!({
                        "type": "function_call",
                        "name": name,
                        "arguments": arguments,
                        "call_id": call_id,
                    }));
                    st.pending_tool_calls.push(PendingFnCall {
                        call_id,
                        name,
                        arguments,
                        item_id,
                        output_index: idx,
                    });
                    st.queue_event(
                        "response.output_item.added",
                        json!({"type": "response.output_item.added", "output_index": idx, "item": item}),
                    );
                }
                Some(crate::agent::StreamEvent::ToolCompleted { call_id, result }) => {
                    let pending = st
                        .pending_tool_calls
                        .iter()
                        .position(|p| p.call_id == call_id)
                        .map(|i| st.pending_tool_calls.remove(i));
                    let Some(pending) = pending else { continue };
                    let done_item = json!({
                        "id": pending.item_id,
                        "type": "function_call",
                        "status": "completed",
                        "name": pending.name,
                        "call_id": pending.call_id,
                        "arguments": pending.arguments,
                    });
                    st.queue_event(
                        "response.output_item.done",
                        json!({"type": "response.output_item.done", "output_index": pending.output_index, "item": done_item}),
                    );
                    let output_parts = json!([{"type": "input_text", "text": result}]);
                    let output_item = json!({
                        "id": format!("fco_{}", uuid::Uuid::new_v4().simple()),
                        "type": "function_call_output",
                        "call_id": pending.call_id,
                        "output": output_parts,
                        "status": "completed",
                    });
                    let idx = st.output_index;
                    st.output_index += 1;
                    st.emitted_items.push(json!({
                        "type": "function_call_output",
                        "call_id": pending.call_id,
                        "output": output_parts,
                    }));
                    st.queue_event(
                        "response.output_item.added",
                        json!({"type": "response.output_item.added", "output_index": idx, "item": output_item}),
                    );
                    st.queue_event(
                        "response.output_item.done",
                        json!({"type": "response.output_item.done", "output_index": idx, "item": output_item}),
                    );
                }
                None => {
                    // Agent finished — emit terminal events.
                    let outcome = match st.guard.handle.take() {
                        Some(handle) => handle.await,
                        None => Ok(Err(AgentError::provider("agent task vanished"))),
                    };
                    let final_text = st.text_parts.join("");
                    if st.message_open {
                        st.queue_event(
                            "response.output_text.done",
                            json!({
                                "type": "response.output_text.done",
                                "item_id": st.message_item_id,
                                "output_index": 0u64.max(st.output_index.saturating_sub(1)),
                                "content_index": 0,
                                "text": final_text,
                                "logprobs": [],
                            }),
                        );
                    }
                    match outcome {
                        Ok(Ok(result)) => {
                            let mut output: Vec<Value> = st.emitted_items.clone();
                            if !final_text.is_empty() {
                                let message_item = json!({
                                    "type": "message",
                                    "id": st.message_item_id,
                                    "role": "assistant",
                                    "status": "completed",
                                    "content": [{"type": "output_text", "text": final_text}],
                                });
                                if st.message_open {
                                    st.queue_event(
                                        "response.output_item.done",
                                        json!({"type": "response.output_item.done", "output_index": 0, "item": message_item}),
                                    );
                                }
                                output.push(message_item);
                            }
                            let mut env = st.envelope("completed");
                            env["output"] = json!(output);
                            env["usage"] = json!({
                                "input_tokens": result.usage.prompt_tokens,
                                "output_tokens": result.usage.completion_tokens,
                                "total_tokens": result.usage.total_tokens,
                            });
                            env["session_id"] = json!(result.session_id);
                            env["previous_response_id"] = json!(st.prev_response_id);
                            st.queue_event(
                                "response.completed",
                                json!({"type": "response.completed", "response": env.clone()}),
                            );
                            st.gateway
                                .responses
                                .lock()
                                .await
                                .insert(st.response_id.clone(), env);
                        }
                        Ok(Err(e)) => {
                            let mut env = st.envelope("failed");
                            env["error"] = json!({"message": e.to_string(), "type": "agent_error"});
                            st.queue_event(
                                "response.failed",
                                json!({"type": "response.failed", "response": env}),
                            );
                        }
                        Err(e) => {
                            let mut env = st.envelope("failed");
                            env["error"] = json!({"message": format!("agent task aborted: {}", e), "type": "agent_error"});
                            st.queue_event(
                                "response.failed",
                                json!({"type": "response.failed", "response": env}),
                            );
                        }
                    }
                    st.finished = true;
                }
            }
        }
    });

    let sse = axum::response::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    );
    let mut response = sse.into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    if let Some(ref sid) = session_id {
        if let Ok(value) = sid.parse() {
            response.headers_mut().insert(SESSION_HEADER, value);
        }
    }
    response
}

async fn create_response(
    State(state): State<Arc<GatewayState>>,
    Json(request): Json<ResponsesRequest>,
) -> Response {
    state
        .metrics
        .responses_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let messages = input_to_messages(&request.input);
    let prompt = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| message_text(&m.content))
        .unwrap_or_default();
    if prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "no user message found in input", "type": "invalid_request_error"}})),
        )
            .into_response();
    }

    // Resolve the session: previous_response_id chains to its session.
    let session_id = if let Some(ref prev) = request.previous_response_id {
        let responses = state.responses.lock().await;
        match responses.get(prev) {
            Some(prev_response) => prev_response
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            None => {
                return not_found(&format!("previous response {} not found", prev));
            }
        }
    } else {
        None
    };

    let history = session_id
        .as_ref()
        .map(|sid| {
            state
                .store
                .load_messages(sid)
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m.role != Role::System)
                .collect::<Vec<_>>()
        })
        .filter(|h| !h.is_empty());

    if request.stream {
        return stream_responses_response(
            state,
            prompt,
            history,
            session_id,
            request.previous_response_id,
        );
    }

    let override_model = session_id
        .as_deref()
        .and_then(|sid| session_model_override(&state, sid));
    let outcome = await_with_model_override(
        override_model,
        state.agent.run_with_session(&prompt, history, session_id.as_deref()),
    )
    .await;
    match outcome {
        Ok(result) => {
            state.metrics.record_run(&result.usage, result.tool_calls.len());
            let response_id = format!("resp_{}", uuid::Uuid::new_v4());
            let body = json!({
                "id": response_id,
                "object": "response",
                "created_at": now_secs() as u64,
                "model": state.model_name,
                "status": "completed",
                "output": [{
                    "type": "message",
                    "id": format!("msg_{}", uuid::Uuid::new_v4()),
                    "role": "assistant",
                    "status": "completed",
                    "content": [{"type": "output_text", "text": result.content}],
                }],
                "usage": {
                    "input_tokens": result.usage.prompt_tokens,
                    "output_tokens": result.usage.completion_tokens,
                    "total_tokens": result.usage.prompt_tokens + result.usage.completion_tokens,
                },
                "session_id": result.session_id,
                "previous_response_id": request.previous_response_id,
            });
            state
                .responses
                .lock()
                .await
                .insert(response_id.clone(), body.clone());
            (StatusCode::CREATED, Json(body)).into_response()
        }
        Err(e) => server_error(&e.to_string()),
    }
}

async fn get_response(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let responses = state.responses.lock().await;
    match responses.get(&id) {
        Some(body) => Json(body.clone()).into_response(),
        None => not_found(&format!("response {} not found", id)),
    }
}

async fn delete_response(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    let mut responses = state.responses.lock().await;
    match responses.remove(&id) {
        Some(_) => Json(json!({"deleted": id})).into_response(),
        None => not_found(&format!("response {} not found", id)),
    }
}

// ---------------------------------------------------------------------------
// Session management API
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionsQuery {
    limit: Option<usize>,
}

/// Attach the owning project slug (longest-prefix folder match on the
/// session cwd via `projects.db`) to session rows — the desktop sidebar
/// renders it as a project badge (hermes desktop session grouping by
/// project). Best-effort: a closed/missing projects store leaves rows
/// with `project: null` instead of failing the listing.
fn enrich_sessions_with_projects(
    rows: Vec<crate::session::sqlite::SessionRow>,
) -> Vec<Value> {
    let cwds: Vec<String> = rows
        .iter()
        .filter_map(|row| row.cwd.clone())
        .filter(|cwd| !cwd.trim().is_empty())
        .collect();
    let mapping = crate::projects_db::connect(None)
        .ok()
        .and_then(|conn| crate::projects_db::projects_for_paths(&conn, &cwds).ok());
    rows.into_iter()
        .map(|row| {
            let mut value = serde_json::to_value(&row).unwrap_or(Value::Null);
            let slug = row
                .cwd
                .as_ref()
                .and_then(|cwd| mapping.as_ref().and_then(|m| m.get(cwd)))
                .map(|project| project.slug.clone());
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "project".to_string(),
                    slug.map(Value::String).unwrap_or(Value::Null),
                );
            }
            value
        })
        .collect()
}

async fn list_sessions(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<SessionsQuery>,
) -> Response {
    let limit = query.limit.unwrap_or(50).min(500);
    match state.store.list_session_rows(limit) {
        Ok(rows) => {
            let data = enrich_sessions_with_projects(rows);
            Json(json!({"object": "list", "data": data})).into_response()
        }
        Err(e) => server_error(&e.to_string()),
    }
}

async fn create_session(State(state): State<Arc<GatewayState>>) -> Response {
    match state
        .store
        .create_session("gateway", Some(&state.model_name), None)
    {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id, "source": "gateway"}))).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn get_session(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    match state.store.get_session_row(&id) {
        Ok(Some(row)) => {
            let mut rows = enrich_sessions_with_projects(vec![row]);
            Json(rows.pop().unwrap_or(Value::Null)).into_response()
        }
        Ok(None) => not_found(&format!("session {} not found", id)),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn delete_session(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    match state.store.get_session_row(&id) {
        Ok(Some(_)) => match state.store.delete_session(&id) {
            Ok(()) => Json(json!({"deleted": id})).into_response(),
            Err(e) => server_error(&e.to_string()),
        },
        Ok(None) => not_found(&format!("session {} not found", id)),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn session_messages(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    match state.store.load_messages(&id) {
        Ok(messages) => Json(json!({"object": "list", "session_id": id, "data": messages})).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

/// Query parameters for `GET /api/sessions/:id/export`.
#[derive(Debug, Deserialize)]
struct ExportQuery {
    /// `md` (default) or `html`.
    format: Option<String>,
}

/// `GET /api/sessions/:id/export?format=md|html` — download the session
/// transcript as a Markdown or standalone HTML file (desktop session
/// export actions; ulnclaw extension).
async fn export_session(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let row = match state.store.get_session_row(&id) {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&format!("session {} not found", id)),
        Err(e) => return server_error(&e.to_string()),
    };
    let messages = match state.store.load_messages_with_timestamps(&id) {
        Ok(messages) => messages,
        Err(e) => return server_error(&e.to_string()),
    };
    let stem = crate::session_export::export_stem(&row);
    let html = matches!(query.format.as_deref(), Some("html"));
    let (mime, extension, body) = if html {
        (
            "text/html; charset=utf-8",
            "html",
            crate::session_export::render_html(&row, &messages),
        )
    } else {
        (
            "text/markdown; charset=utf-8",
            "md",
            crate::session_export::render_markdown(&row, &messages),
        )
    };
    let filename = format!("ulnclaw-session-{stem}.{extension}");
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response()
}

/// `POST /api/uploads` — store a binary upload (desktop composer pastes
/// clipboard images here) in the content-addressed media cache and hand
/// back the path reference. The agent inspects it with
/// vision_analyze/read_file — hermes' text-fallback media semantics for
/// surfaces without native multimodal injection.
// ── Backup snapshots (hermes /api/ops/backup parity) ───────────────────────

/// `GET /api/backups` — quick-snapshot inventory (`ulnclaw backup list`).
async fn list_backups(State(state): State<Arc<GatewayState>>) -> Response {
    let home = state.agent.context().home.clone();
    let snapshots = tokio::task::spawn_blocking(move || crate::backup::list_quick_snapshots(&home))
        .await
        .unwrap_or_default();
    Json(json!({
        "object": "ulnclaw.backup_list",
        "snapshots": snapshots
            .iter()
            .map(|s| json!({"id": s.id, "files": s.files, "bytes": s.bytes}))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(serde::Deserialize, Default)]
struct CreateBackupBody {
    #[serde(default)]
    label: Option<String>,
}

/// `POST /api/backups` — create a quick state snapshot
/// (`ulnclaw backup --quick`): critical files only (config, state.db,
/// .env, cron, skills usage, memory).
async fn create_backup(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CreateBackupBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let label = body.label.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::backup::create_quick_snapshot(&home, label.as_deref(), None, None)
    })
    .await;
    match result {
        Ok(Ok(Some(id))) => Json(json!({"object": "ulnclaw.backup", "id": id})).into_response(),
        Ok(Ok(None)) => Json(json!({
            "object": "ulnclaw.backup",
            "id": null,
            "message": "No state files found to snapshot.",
        }))
        .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("backup task failed: {e}")})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct PruneBackupsBody {
    #[serde(default)]
    keep: Option<usize>,
}

/// `POST /api/backups/prune` — keep only the newest N snapshots
/// (`ulnclaw backup prune [keep]`, default 20).
async fn prune_backups(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<PruneBackupsBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let keep = body.keep.unwrap_or(crate::backup::QUICK_DEFAULT_KEEP);
    let removed =
        tokio::task::spawn_blocking(move || crate::backup::prune_quick_snapshots(&home, keep))
            .await
            .unwrap_or(0);
    Json(json!({"object": "ulnclaw.backup_prune", "removed": removed, "keep": keep})).into_response()
}

/// `POST /api/backups/:id/restore` — overlay a snapshot onto the home
/// (`ulnclaw backup restore <id>`). 404 when the snapshot is unknown.
async fn restore_backup(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let home = state.agent.context().home.clone();
    let snapshot_id = id.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::backup::restore_quick_snapshot(&home, &snapshot_id)
    })
    .await;
    match result {
        Ok(Ok(true)) => Json(json!({"object": "ulnclaw.backup_restore", "restored": true})).into_response(),
        Ok(Ok(false)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("snapshot '{id}' not found or empty")})),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("restore task failed: {e}")})),
        )
            .into_response(),
    }
}

// ── Curator (hermes curator CLI parity) ────────────────────────────────────

/// `GET /api/curator` — curation overview: status summary, archived
/// skill names, and the usage telemetry table sorted by activity
/// (`ulnclaw curator status` + `list-archived` + `usage --json`).
async fn curator_status(State(state): State<Arc<GatewayState>>) -> Response {
    let home = state.agent.context().home.clone();
    let payload = tokio::task::spawn_blocking(move || -> Value {
        let status = crate::curator::status_summary(&home)
            .into_iter()
            .map(|(label, count)| json!({"label": label, "count": count}))
            .collect::<Vec<_>>();
        let archived = crate::skill_usage::list_archived_skill_names(&home);
        let mut rows = crate::skill_usage::usage_report(&home);
        rows.sort_by(|a, b| b.activity_count.cmp(&a.activity_count).then(a.name.cmp(&b.name)));
        let usage = rows
            .iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "provenance": r.provenance,
                    "use_count": r.use_count,
                    "view_count": r.view_count,
                    "patch_count": r.patch_count,
                    "activity_count": r.activity_count,
                    "last_activity_at": r.last_activity_at,
                    "state": r.state,
                    "pinned": r.pinned,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "object": "ulnclaw.curator",
            "status": status,
            "archived": archived,
            "usage": usage,
        })
    })
    .await
    .unwrap_or_else(|e| json!({"error": format!("curator task failed: {e}")}));
    axum::Json(payload).into_response()
}

#[derive(serde::Deserialize)]
struct CuratorSkillBody {
    skill: String,
}

/// `POST /api/curator/pin` — pin a skill so auto-transitions skip it
/// (`ulnclaw curator pin <skill>`).
async fn curator_pin(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CuratorSkillBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let skill = body.skill.clone();
    tokio::task::spawn_blocking(move || crate::skill_usage::set_pinned(&home, &skill, true))
        .await
        .ok();
    Json(json!({"ok": true, "skill": body.skill, "pinned": true})).into_response()
}

/// `POST /api/curator/unpin` — unpin a skill (`ulnclaw curator unpin`).
async fn curator_unpin(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CuratorSkillBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let skill = body.skill.clone();
    tokio::task::spawn_blocking(move || crate::skill_usage::set_pinned(&home, &skill, false))
        .await
        .ok();
    Json(json!({"ok": true, "skill": body.skill, "pinned": false})).into_response()
}

/// `POST /api/curator/archive` — archive a skill now, recoverable via
/// restore (`ulnclaw curator archive`). Pinned skills are refused.
async fn curator_archive(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CuratorSkillBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let skill = body.skill.clone();
    let result = tokio::task::spawn_blocking(move || {
        if crate::skill_usage::get_record(&home, &skill)
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return (false, format!("'{skill}' is pinned — unpin it first"));
        }
        crate::skill_usage::archive_skill(&home, &skill)
    })
    .await;
    match result {
        Ok((true, message)) => {
            Json(json!({"ok": true, "skill": body.skill, "message": message})).into_response()
        }
        Ok((false, message)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": message})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("archive task failed: {e}")})),
        )
            .into_response(),
    }
}

/// `POST /api/curator/restore` — restore an archived skill
/// (`ulnclaw curator restore`).
async fn curator_restore(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CuratorSkillBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let skill = body.skill.clone();
    let result =
        tokio::task::spawn_blocking(move || crate::skill_usage::restore_skill(&home, &skill))
            .await;
    match result {
        Ok((true, message)) => {
            Json(json!({"ok": true, "skill": body.skill, "message": message})).into_response()
        }
        Ok((false, message)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": message})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("restore task failed: {e}")})),
        )
            .into_response(),
    }
}

// ── Checkpoints (hermes checkpoints CLI parity) ────────────────────────────

/// `GET /api/checkpoints/status` — shared checkpoint store status
/// (projects, sizes; `ulnclaw checkpoint status`).
async fn checkpoints_status(State(state): State<Arc<GatewayState>>) -> Response {
    let home = state.agent.context().home.clone();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let manager =
        crate::checkpoint::CheckpointManager::new(home.join("checkpoints"), &config.checkpoints);
    let status = manager.status().await;
    Json(json!({"object": "ulnclaw.checkpoint_status", "status": status})).into_response()
}

#[derive(serde::Deserialize)]
struct CheckpointsListQuery {
    #[serde(default)]
    dir: Option<String>,
}

/// `GET /api/checkpoints?dir=` — checkpoint list for a working
/// directory (`ulnclaw checkpoint list --dir`).
async fn checkpoints_list(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<CheckpointsListQuery>,
) -> Response {
    let Some(dir) = query.dir.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()) else {
        return bad_request("dir is required", Some("invalid_request"));
    };
    let home = state.agent.context().home.clone();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let manager =
        crate::checkpoint::CheckpointManager::new(home.join("checkpoints"), &config.checkpoints);
    let checkpoints = manager.list_checkpoints(&dir).await;
    Json(json!({
        "object": "ulnclaw.checkpoint_list",
        "dir": dir,
        "checkpoints": checkpoints,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct CheckpointRestoreBody {
    dir: String,
    hash: String,
    #[serde(default)]
    file: Option<String>,
}

/// `POST /api/checkpoints/restore` — restore a directory (or a single
/// file) to a checkpoint (`ulnclaw checkpoint restore <hash>`).
async fn checkpoints_restore(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CheckpointRestoreBody>,
) -> Response {
    if body.dir.trim().is_empty() || body.hash.trim().is_empty() {
        return bad_request("dir and hash are required", Some("invalid_request"));
    }
    let home = state.agent.context().home.clone();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let manager =
        crate::checkpoint::CheckpointManager::new(home.join("checkpoints"), &config.checkpoints);
    match manager.restore(&body.dir, &body.hash, body.file.as_deref()).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct CheckpointPruneBody {
    #[serde(default)]
    days: Option<u64>,
}

/// `POST /api/checkpoints/prune` — drop orphan/stale checkpoints and
/// reclaim store space (`ulnclaw checkpoint prune`).
async fn checkpoints_prune(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CheckpointPruneBody>,
) -> Response {
    let home = state.agent.context().home.clone();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let days = body.days.unwrap_or(config.checkpoints.retention_days);
    let manager =
        crate::checkpoint::CheckpointManager::new(home.join("checkpoints"), &config.checkpoints);
    let stats = manager.prune(days, true).await;
    Json(json!({"object": "ulnclaw.checkpoint_prune", "stats": stats, "days": days})).into_response()
}

// ── Learning graph (hermes web_server /api/learning/* parity) ─────────────

/// GET /api/learning/graph — learned skills + memory chunks with graph
/// edges (desktop "star map" / journey panel).
async fn learning_graph(State(state): State<Arc<GatewayState>>) -> Response {
    let home = state.agent.context().home.clone();
    let payload = tokio::task::spawn_blocking(move || crate::learning_graph::build_learning_graph(&home))
        .await
        .unwrap_or_else(|e| json!({"error": format!("learning graph task failed: {e}")}));
    axum::Json(payload).into_response()
}

#[derive(serde::Deserialize)]
struct LearningNodeQuery {
    id: String,
}

/// GET /api/learning/node?id= — node content for an edit prefill.
async fn learning_node_get(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<LearningNodeQuery>,
) -> Response {
    let home = state.agent.context().home.clone();
    let id = params.id.clone();
    let result = tokio::task::spawn_blocking(move || crate::learning_mutations::node_detail(&home, &id))
        .await
        .unwrap_or_else(|e| json!({"ok": false, "message": format!("task failed: {e}")}));
    if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        axum::Json(result).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(result),
        )
            .into_response()
    }
}

#[derive(serde::Deserialize)]
struct LearningNodeRef {
    id: String,
}

#[derive(serde::Deserialize)]
struct LearningNodeEdit {
    id: String,
    content: String,
}

/// DELETE /api/learning/node — archive a learned skill or remove a
/// memory chunk.
async fn learning_node_delete(
    State(state): State<Arc<GatewayState>>,
    axum::Json(body): axum::Json<LearningNodeRef>,
) -> Response {
    let home = state.agent.context().home.clone();
    let id = body.id.clone();
    let result = tokio::task::spawn_blocking(move || crate::learning_mutations::delete_node(&home, &id))
        .await
        .unwrap_or_else(|e| json!({"ok": false, "message": format!("task failed: {e}")}));
    if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        axum::Json(result).into_response()
    } else {
        (StatusCode::BAD_REQUEST, axum::Json(result)).into_response()
    }
}

/// PUT /api/learning/node — rewrite a node's content.
async fn learning_node_put(
    State(state): State<Arc<GatewayState>>,
    axum::Json(body): axum::Json<LearningNodeEdit>,
) -> Response {
    let home = state.agent.context().home.clone();
    let id = body.id.clone();
    let content = body.content.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::learning_mutations::edit_node(&home, &id, &content)
    })
    .await
    .unwrap_or_else(|e| json!({"ok": false, "message": format!("task failed: {e}")}));
    if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        axum::Json(result).into_response()
    } else {
        (StatusCode::BAD_REQUEST, axum::Json(result)).into_response()
    }
}

async fn upload_media(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024; // hermes media cap
    if body.is_empty() {
        return bad_request("empty upload body", None);
    }
    if body.len() > MAX_UPLOAD_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": {
                "message": "upload exceeds the 25 MB media cap",
                "type": "invalid_request_error"
            }})),
        )
            .into_response();
    }
    let mime = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let name_hint = params.get("name").cloned().unwrap_or_default();
    let home = state.agent.context().home.clone();
    match crate::media_cache::cache_media_bytes(&home, &body, &mime, &name_hint) {
        Ok(path) => Json(json!({
            "path": path.display().to_string(),
            "mime": crate::media_cache::normalize_mime(&mime),
            "bytes": body.len(),
        }))
        .into_response(),
        Err(e) => server_error(&format!("upload cache failed: {e}")),
    }
}

#[derive(Deserialize)]
struct SessionChatRequest {
    message: String,
}

/// Slash handling for the session chat endpoints (hermes desktop slash
/// passthrough): `/skill-name` + `/<bundle>` invocations expand into
/// scaffolded agent turns (hermes gateway/run.py shares skill_commands
/// with the CLI for exactly this), while a small session-scoped command
/// set runs without an LLM turn.
enum GatewaySlash {
    /// Expanded text to run through the agent as the user turn.
    AgentTurn(String),
    /// Reply directly — no LLM turn.
    Direct(String),
}

const GATEWAY_SLASH_HELP: &str = "Gateway slash commands:
  /help            this list
  /skills          list skills (invoke one: /<skill-name> [instruction])
  /tools           list enabled tools
  /recap           recap this session
  /title [text]    show or set the session title
  /usage           this session's token usage
  /insights [N] [--days N] [--source S]   usage analytics across sessions
  /<bundle>        invoke a skill bundle (ulnclaw bundles)";

async fn resolve_gateway_slash(
    state: &Arc<GatewayState>,
    session_id: &str,
    message: &str,
) -> Option<GatewaySlash> {
    let trimmed = message.trim();
    let stripped = trimmed.strip_prefix('/')?;
    if stripped.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    let skills_dir = state.agent.context().home.join("skills");
    match cmd {
        "/help" => Some(GatewaySlash::Direct(GATEWAY_SLASH_HELP.to_string())),
        "/skills" => {
            let skills = crate::skills::list_skills(&skills_dir);
            if skills.is_empty() {
                Some(GatewaySlash::Direct(
                    "no skills installed (<home>/skills).".to_string(),
                ))
            } else {
                let mut text = String::new();
                for skill in &skills {
                    text.push_str(&format!("  {} — {}\n", skill.name, skill.description));
                }
                Some(GatewaySlash::Direct(text.trim_end().to_string()))
            }
        }
        "/tools" => Some(GatewaySlash::Direct(state.agent.tool_names().join(", "))),
        "/recap" => {
            let row = state.store.get_session_row(session_id).ok().flatten();
            let messages = state.store.load_messages(session_id).unwrap_or_default();
            let recap = crate::session::recap::build_recap(
                &messages,
                row.as_ref().and_then(|r| r.title.as_deref()),
                Some(session_id),
            );
            Some(GatewaySlash::Direct(recap))
        }
        "/title" => {
            if rest.is_empty() {
                let title = state
                    .store
                    .get_session_row(session_id)
                    .ok()
                    .flatten()
                    .and_then(|row| row.title.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "(untitled)".to_string());
                Some(GatewaySlash::Direct(format!("title: {title}")))
            } else {
                match state.store.set_session_title(session_id, rest) {
                    Ok(()) => Some(GatewaySlash::Direct(format!("title set: {rest}"))),
                    Err(e) => Some(GatewaySlash::Direct(format!("title set failed: {e}"))),
                }
            }
        }
        "/usage" => {
            let Some(row) = state.store.get_session_row(session_id).ok().flatten() else {
                return Some(GatewaySlash::Direct("session not found.".to_string()));
            };
            Some(GatewaySlash::Direct(format!(
                "messages: {}  tokens: {} in / {} out",
                row.message_count, row.input_tokens, row.output_tokens
            )))
        }
        "/insights" => {
            let mut days: u32 = 30;
            let mut source: Option<String> = None;
            let mut tokens = rest.split_whitespace();
            while let Some(token) = tokens.next() {
                if token == "--days" {
                    if let Some(value) = tokens.next() {
                        if let Ok(parsed) = value.parse::<u32>() {
                            days = parsed;
                        }
                    }
                } else if token == "--source" {
                    source = tokens.next().map(String::from);
                } else if let Ok(parsed) = token.parse::<u32>() {
                    days = parsed;
                }
            }
            let provider_hint = state.provider_name.as_str();
            let result = match crate::insights::InsightsEngine::open_default() {
                Ok(engine) => match engine.generate(days, source.as_deref(), Some(provider_hint)) {
                    Ok(report) => crate::insights::format_gateway(&report),
                    Err(e) => format!("insights failed: {e}"),
                },
                Err(e) => format!("insights failed: {e}"),
            };
            Some(GatewaySlash::Direct(result))
        }
        _ => {
            let cmd_name = cmd.trim_start_matches('/');
            // Bundles win over single skills (hermes bundle-over-skill
            // slash precedence).
            if let Some(key) = crate::bundles::resolve_bundle_command_key(cmd_name) {
                if let Some((message, _loaded, _missing)) =
                    crate::bundles::build_bundle_invocation_message(&key, rest, &skills_dir)
                {
                    return Some(GatewaySlash::AgentTurn(message));
                }
            }
            if let Some(message) =
                crate::skills::build_skill_invocation_message(&skills_dir, cmd_name, rest)
            {
                return Some(GatewaySlash::AgentTurn(message));
            }
            Some(GatewaySlash::Direct(format!(
                "unknown command: {cmd} — /help lists gateway slash commands"
            )))
        }
    }
}

/// Persist a direct slash exchange so the transcript stays whole, and
/// answer with a single-chunk SSE stream (chat-completions shape).
fn direct_sse_response(state: &Arc<GatewayState>, session_id: &str, request: &str, text: String) -> Response {
    let user_msg = Message {
        role: Role::User,
        content: Some(request.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    };
    let assistant_msg = Message {
        role: Role::Assistant,
        content: Some(text.clone()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    };
    let _ = state.store.append_message(session_id, &user_msg);
    let _ = state.store.append_message(session_id, &assistant_msg);
    use axum::response::sse::{Event, Sse};
    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = now_secs() as u64;
    let model = state.model_name.clone();
    let chunk = |delta: Value, finish: Value| -> Value {
        json!({
            "id": completion_id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
    };
    let mut final_chunk = chunk(json!({}), json!("stop"));
    final_chunk["usage"] =
        json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0});
    let events: Vec<Event> = vec![
        Event::default().json_data(chunk(json!({"role": "assistant"}), Value::Null)).unwrap(),
        Event::default().json_data(chunk(json!({"content": text}), Value::Null)).unwrap(),
        Event::default().json_data(final_chunk).unwrap(),
        Event::default().data("[DONE]"),
    ];
    let stream = futures::stream::iter(
        events
            .into_iter()
            .map(Ok::<Event, std::convert::Infallible>),
    );
    Sse::new(stream).into_response()
}

/// Drain finished background delegations into a session before its turn —
/// hermes' async-delegation wake delivery (positive-ownership by the
/// process session key; ulnclaw runs one profile per gateway process).
fn drain_delegations_into_session(state: &GatewayState, session_id: &str) {
    let key = state.agent.context().session_id.clone();
    for completion in
        crate::async_delegation::drain_completions(Some(&state.store), &key)
    {
        let message = crate::provider::Message {
            role: crate::provider::Role::User,
            content: Some(completion.message),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };
        let _ = state.store.append_message(session_id, &message);
    }
}

async fn session_chat(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(request): Json<SessionChatRequest>,
) -> Response {
    if request.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "message is required", "type": "invalid_request_error"}})),
        )
            .into_response();
    }
    drain_delegations_into_session(&state, &id);
    let mut message = request.message.clone();
    if let Some(slash) = resolve_gateway_slash(&state, &id, &message).await {
        match slash {
            GatewaySlash::Direct(text) => {
                let user_msg = Message {
                    role: Role::User,
                    content: Some(message.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                };
                let assistant_msg = Message {
                    role: Role::Assistant,
                    content: Some(text.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                };
                let _ = state.store.append_message(&id, &user_msg);
                let _ = state.store.append_message(&id, &assistant_msg);
                return Json(json!({
                    "session_id": id,
                    "response": text,
                    "iterations": 0,
                }))
                .into_response();
            }
            GatewaySlash::AgentTurn(expanded) => message = expanded,
        }
    }
    let history = state
        .store
        .load_messages(&id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role != Role::System)
        .collect::<Vec<_>>();
    let history_arg = if history.is_empty() { None } else { Some(history) };
    let override_model = session_model_override(&state, &id);
    state
        .metrics
        .session_chats
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let outcome = await_with_model_override(
        override_model,
        state.agent.run_with_session(&message, history_arg, Some(&id)),
    )
    .await;
    match outcome {
        Ok(result) => {
            state.metrics.record_run(&result.usage, result.tool_calls.len());
            Json(json!({
                "session_id": id,
                "response": result.content,
                "iterations": result.iterations,
            }))
            .into_response()
        }
        Err(e) => server_error(&e.to_string()),
    }
}

/// `POST /api/sessions/:id/chat/stream` — SSE wrapper over session chat
/// (hermes `_handle_session_chat_stream`).
async fn session_chat_stream(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(request): Json<SessionChatRequest>,
) -> Response {
    if request.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "message is required", "type": "invalid_request_error"}})),
        )
            .into_response();
    }
    drain_delegations_into_session(&state, &id);
    let mut message = request.message.clone();
    if let Some(slash) = resolve_gateway_slash(&state, &id, &message).await {
        match slash {
            GatewaySlash::Direct(text) => {
                return direct_sse_response(&state, &id, &message, text);
            }
            GatewaySlash::AgentTurn(expanded) => message = expanded,
        }
    }
    let history = state
        .store
        .load_messages(&id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role != Role::System)
        .collect::<Vec<_>>();
    let history_arg = if history.is_empty() { None } else { Some(history) };
    stream_agent_response(state, message, history_arg, Some(id))
}

/// `PATCH /api/sessions/:id` — update client-safe session metadata
/// (hermes `_handle_patch_session`). Only `title` and `end_reason` are
/// accepted; unknown fields are rejected.
async fn patch_session(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match state.store.get_session_row(&id) {
        Ok(None) => return not_found(&format!("session {} not found", id)),
        Err(e) => return server_error(&e.to_string()),
        Ok(Some(_)) => {}
    }
    let Some(obj) = body.as_object() else {
        return bad_request("body must be a JSON object", None);
    };
    let unknown: Vec<&String> = obj
        .keys()
        .filter(|k| k.as_str() != "title" && k.as_str() != "end_reason")
        .collect();
    if !unknown.is_empty() {
        let names = unknown
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return bad_request(
            &format!("Unsupported session fields: {}", names),
            Some("unsupported_session_field"),
        );
    }
    if let Some(title) = obj.get("title") {
        let title = match title {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if let Err(e) = state.store.set_session_title(&id, &title) {
            return bad_request(&e.to_string(), Some("invalid_title"));
        }
    }
    if let Some(reason) = obj.get("end_reason").and_then(|v| v.as_str()) {
        if !reason.is_empty() {
            if let Err(e) = state.store.end_session(&id, reason) {
                return server_error(&e.to_string());
            }
        }
    }
    match state.store.get_session_row(&id) {
        Ok(Some(row)) => Json(json!({"object": "ulnclaw.session", "session": row})).into_response(),
        Ok(None) => not_found(&format!("session {} not found", id)),
        Err(e) => server_error(&e.to_string()),
    }
}

/// `POST /api/sessions/:id/fork` — branch a session (hermes
/// `_handle_fork_session`). Marks the source as `branched` and creates a
/// child session carrying the transcript forward.
async fn fork_session(
    State(state): State<Arc<GatewayState>>,
    Path(source_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let source = match state.store.get_session_row(&source_id) {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&format!("session {} not found", source_id)),
        Err(e) => return server_error(&e.to_string()),
    };
    let fork_id = body
        .get("id")
        .or_else(|| body.get("session_id"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            format!(
                "api_{}_{}",
                now_secs() as u64,
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            )
        });
    if fork_id.contains(['\r', '\n', '\0']) {
        return bad_request("Invalid session ID", Some("invalid_session_id"));
    }
    match state.store.get_session_row(&fork_id) {
        Ok(Some(_)) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error": {"message": format!("Session already exists: {}", fork_id), "type": "invalid_request_error", "code": "session_exists"}})),
            )
                .into_response()
        }
        Err(e) => return server_error(&e.to_string()),
        Ok(None) => {}
    }
    if let Err(e) = state.store.end_session(&source_id, "branched") {
        return server_error(&e.to_string());
    }
    if let Err(e) = state.store.create_named_session(
        &fork_id,
        "gateway",
        source.model.as_deref(),
        Some(&source_id),
    ) {
        return server_error(&e.to_string());
    }
    match state.store.load_messages(&source_id) {
        Ok(messages) => {
            for message in &messages {
                if let Err(e) = state.store.append_message(&fork_id, message) {
                    return server_error(&e.to_string());
                }
            }
        }
        Err(e) => return server_error(&e.to_string()),
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .unwrap_or_else(|| {
            format!("{} fork", source.title.as_deref().unwrap_or("fork"))
        });
    if let Err(e) = state.store.set_session_title(&fork_id, &title) {
        return bad_request(&e.to_string(), Some("invalid_title"));
    }
    match state.store.get_session_row(&fork_id) {
        Ok(Some(row)) => (
            StatusCode::CREATED,
            Json(json!({"object": "ulnclaw.session", "session": row})),
        )
            .into_response(),
        Ok(None) => server_error("fork created but not found"),
        Err(e) => server_error(&e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Cron jobs API (/api/jobs — hermes `_handle_*_job` parity)
// ---------------------------------------------------------------------------

const MAX_JOB_NAME_LENGTH: usize = 200;
const MAX_JOB_PROMPT_LENGTH: usize = 5000;

fn jobs_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn cron_store(state: &GatewayState) -> std::result::Result<Arc<CronStore>, Response> {
    state.cron.get().cloned().ok_or_else(|| {
        jobs_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "cron jobs are not enabled on this gateway",
        )
    })
}

fn job_value(job: &CronJob) -> Value {
    serde_json::to_value(job).unwrap_or(Value::Null)
}

#[derive(Deserialize)]
struct JobsQuery {
    #[serde(default)]
    include_disabled: Option<String>,
}

async fn list_jobs(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<JobsQuery>,
) -> Response {
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let include_disabled = query
        .include_disabled
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);
    match store.list() {
        Ok(jobs) => {
            let jobs: Vec<Value> = jobs
                .into_iter()
                .filter(|job| include_disabled || job.enabled)
                .map(|job| job_value(&job))
                .collect();
            Json(json!({"jobs": jobs})).into_response()
        }
        Err(e) => server_error(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct CreateJobRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    /// `Value` (not `String`): hermes accepts lists here too and
    /// `_normalize_deliver_value` flattens them to a comma list.
    #[serde(default)]
    deliver: Option<Value>,
    #[serde(default)]
    skills: Option<Vec<String>>,
    #[serde(default)]
    repeat: Option<i64>,
}

fn validate_job_fields(name: &str, prompt: &str) -> std::result::Result<(), Response> {
    if name.is_empty() {
        return Err(jobs_error(StatusCode::BAD_REQUEST, "Name is required"));
    }
    if name.chars().count() > MAX_JOB_NAME_LENGTH {
        return Err(jobs_error(
            StatusCode::BAD_REQUEST,
            &format!("Name must be ≤ {} characters", MAX_JOB_NAME_LENGTH),
        ));
    }
    if prompt.chars().count() > MAX_JOB_PROMPT_LENGTH {
        return Err(jobs_error(
            StatusCode::BAD_REQUEST,
            &format!("Prompt must be ≤ {} characters", MAX_JOB_PROMPT_LENGTH),
        ));
    }
    Ok(())
}

async fn create_job(State(state): State<Arc<GatewayState>>, Json(request): Json<CreateJobRequest>) -> Response {
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let name = request.name.unwrap_or_default().trim().to_string();
    let schedule = request.schedule.unwrap_or_default().trim().to_string();
    let prompt = request.prompt.unwrap_or_default();
    if schedule.is_empty() {
        return jobs_error(StatusCode::BAD_REQUEST, "Schedule is required");
    }
    if let Err(response) = validate_job_fields(&name, &prompt) {
        return response;
    }
    if let Some(repeat) = request.repeat {
        if repeat < 1 {
            return jobs_error(
                StatusCode::BAD_REQUEST,
                "Repeat must be a positive integer",
            );
        }
    }
    // `deliver` is stored verbatim (normalized) and resolved at fire
    // time — hermes `create_job` accepts any target string and lets
    // resolution fail soft at run time.
    let deliver = crate::cron::delivery::normalize_deliver_value(request.deliver.as_ref());
    let parsed = match crate::cron::parse_schedule(&schedule) {
        Ok(parsed) => parsed,
        Err(e) => {
            return jobs_error(
                StatusCode::BAD_REQUEST,
                &format!("Invalid schedule: {}", e),
            )
        }
    };
    let job = CronJob {
        id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        name,
        schedule,
        prompt,
        skills: request.skills.unwrap_or_default(),
        enabled: true,
        repeat: request.repeat,
        next_run: crate::cron::next_run(&parsed),
        created_at: now_secs(),
        last_run: None,
        last_status: None,
        deliver: Some(deliver),
        origin: None,
        last_delivery_error: None,
    };
    match store.add(&job) {
        Ok(()) => Json(json!({"job": job_value(&job)})).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn get_job(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    match store.get(&id) {
        Ok(Some(job)) => Json(json!({"job": job_value(&job)})).into_response(),
        Ok(None) => jobs_error(StatusCode::NOT_FOUND, "Job not found"),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn update_job(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let mut job = match store.get(&id) {
        Ok(Some(job)) => job,
        Ok(None) => return jobs_error(StatusCode::NOT_FOUND, "Job not found"),
        Err(e) => return server_error(&e.to_string()),
    };
    let Some(obj) = body.as_object() else {
        return jobs_error(StatusCode::BAD_REQUEST, "body must be a JSON object");
    };
    // Whitelist of mutable fields (hermes blocks only the immutable
    // fields; this port whitelists the mutable ones, incl. `deliver`
    // since P219 persists it).
    let allowed = [
        "name",
        "schedule",
        "prompt",
        "skills",
        "repeat",
        "enabled",
        "deliver",
    ];
    let updates: Vec<(&String, &Value)> = obj
        .iter()
        .filter(|(key, _)| allowed.contains(&key.as_str()))
        .collect();
    if updates.is_empty() {
        return jobs_error(StatusCode::BAD_REQUEST, "No valid fields to update");
    }
    for (key, value) in updates {
        match key.as_str() {
            "name" => {
                let name = value.as_str().unwrap_or("").trim().to_string();
                if let Err(response) = validate_job_fields(&name, &job.prompt) {
                    return response;
                }
                job.name = name;
            }
            "prompt" => {
                let prompt = value.as_str().unwrap_or("").to_string();
                if let Err(response) = validate_job_fields(&job.name, &prompt) {
                    return response;
                }
                job.prompt = prompt;
            }
            "schedule" => {
                let schedule = value.as_str().unwrap_or("").trim().to_string();
                if schedule.is_empty() {
                    return jobs_error(StatusCode::BAD_REQUEST, "Schedule is required");
                }
                match crate::cron::parse_schedule(&schedule) {
                    Ok(parsed) => {
                        job.next_run = crate::cron::next_run(&parsed);
                        job.schedule = schedule;
                    }
                    Err(e) => {
                        return jobs_error(
                            StatusCode::BAD_REQUEST,
                            &format!("Invalid schedule: {}", e),
                        )
                    }
                }
            }
            "skills" => {
                if let Some(list) = value.as_array() {
                    job.skills = list
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            "repeat" => match value {
                Value::Null => job.repeat = None,
                Value::Number(n) if n.as_i64().map(|v| v >= 1).unwrap_or(false) => {
                    job.repeat = n.as_i64();
                }
                _ => {
                    return jobs_error(
                        StatusCode::BAD_REQUEST,
                        "Repeat must be a positive integer",
                    )
                }
            },
            "enabled" => {
                if let Some(flag) = value.as_bool() {
                    job.enabled = flag;
                    if flag {
                        if let Ok(parsed) = crate::cron::parse_schedule(&job.schedule) {
                            job.next_run = crate::cron::next_run(&parsed);
                        }
                    } else {
                        job.next_run = None;
                    }
                }
            }
            "deliver" => {
                let deliver = crate::cron::delivery::normalize_deliver_value(Some(value));
                job.deliver = Some(deliver);
            }
            _ => {}
        }
    }
    match store.update(&job) {
        Ok(()) => Json(json!({"job": job_value(&job)})).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn delete_job(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    match store.remove(&id) {
        Ok(true) => Json(json!({"ok": true})).into_response(),
        Ok(false) => jobs_error(StatusCode::NOT_FOUND, "Job not found"),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn set_job_enabled(state: &GatewayState, id: &str, enabled: bool) -> Response {
    let store = match cron_store(state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let mut job = match store.get(id) {
        Ok(Some(job)) => job,
        Ok(None) => return jobs_error(StatusCode::NOT_FOUND, "Job not found"),
        Err(e) => return server_error(&e.to_string()),
    };
    job.enabled = enabled;
    job.next_run = if enabled {
        crate::cron::parse_schedule(&job.schedule)
            .ok()
            .and_then(|parsed| crate::cron::next_run(&parsed))
    } else {
        None
    };
    match store.update(&job) {
        Ok(()) => Json(json!({"job": job_value(&job)})).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

async fn pause_job(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    set_job_enabled(&state, &id, false).await
}

async fn resume_job(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    set_job_enabled(&state, &id, true).await
}

/// `GET /api/jobs/delivery-targets` — delivery targets the dropdown
/// should offer (hermes `GET /api/cron/delivery-targets`): always the
/// implicit `local` option plus every connected gateway platform, with
/// `home_target_set` so the UI can prompt for a home channel.
async fn job_delivery_targets() -> Response {
    let mut targets = vec![json!({
        "id": "local",
        "name": "Local (save only)",
        "home_target_set": true,
        "home_env_var": Value::Null,
    })];
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let connected = crate::cron::delivery::connected_messaging_platforms(&config.messaging);
    targets.extend(crate::cron::delivery::cron_delivery_targets(&connected));
    Json(json!({ "targets": targets })).into_response()
}

/// `POST /api/jobs/fire` — Chronos managed-cron fire webhook (NAS →
/// agent), hermes `POST /api/cron/fire` parity. Authenticated by a
/// short-lived NAS-minted JWT (the route is public — the JWT is the
/// gate, not the gateway bearer key). Verifies, claims the job, returns
/// 202 immediately and runs the job in the background so a long agent
/// turn never trips NAS's HTTP timeout.
async fn fire_job(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .trim()
                .strip_prefix("Bearer ")
                .unwrap_or(value.trim())
                .to_string()
        })
        .unwrap_or_default();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let chronos = &config.cron.chronos;
    let claims = crate::cron::chronos::verify_fire_token(
        &token,
        chronos.expected_audience.as_deref().unwrap_or(""),
        chronos.nas_jwks_url.as_deref(),
        chronos.portal_url.as_deref(),
        30,
    )
    .await;
    if claims.is_none() {
        return jobs_error(StatusCode::UNAUTHORIZED, "invalid fire token");
    }
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let job_id = payload
        .get("job_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let Some(job_id) = job_id else {
        return jobs_error(StatusCode::BAD_REQUEST, "missing job_id");
    };
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let mut job = match store.get(job_id) {
        Ok(Some(job)) => job,
        // Job is gone (cancelled / completed) — nothing to fire. 200 so
        // NAS does not retry a fire that is intentionally absent.
        Ok(None) => {
            return Json(json!({ "status": "gone", "job_id": job_id })).into_response()
        }
        Err(e) => return server_error(&e.to_string()),
    };
    // CAS claim: a NAS retry that lands while the previous fire is still
    // running is accepted but not double-dispatched.
    if !take_fire_claim(job_id) {
        return Json(json!({ "status": "accepted", "job_id": job_id })).into_response();
    }
    if job.prompt.trim().is_empty() {
        release_fire_claim(job_id);
        return jobs_error(StatusCode::BAD_REQUEST, "Job has no prompt to run");
    }
    match spawn_job_run(state.clone(), &mut job, store).await {
        Some(_) => (
            StatusCode::ACCEPTED,
            Json(json!({ "status": "accepted", "job_id": job_id })),
        )
            .into_response(),
        None => {
            release_fire_claim(job_id);
            server_error("failed to start job run")
        }
    }
}

/// `POST /api/jobs/:id/run` — trigger one immediate execution as a tracked
/// run (hermes `_handle_run_job`).
async fn run_job_now(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let store = match cron_store(&state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let mut job = match store.get(&id) {
        Ok(Some(job)) => job,
        Ok(None) => return jobs_error(StatusCode::NOT_FOUND, "Job not found"),
        Err(e) => return server_error(&e.to_string()),
    };
    if job.prompt.trim().is_empty() {
        return jobs_error(StatusCode::BAD_REQUEST, "Job has no prompt to run");
    }
    let Some(run_id) = spawn_job_run(state.clone(), &mut job, store).await else {
        return server_error("failed to start job run");
    };
    Json(json!({"job": job_value(&job), "run_id": run_id})).into_response()
}

/// In-flight Chronos fire claims (hermes store CAS claim): dedupes a
/// NAS retry that arrives while a previous fire of the same job is
/// still running.
fn fire_claims() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static CLAIMS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    CLAIMS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Take the fire claim for a job. Returns false when a fire is already
/// in flight for it.
fn take_fire_claim(job_id: &str) -> bool {
    fire_claims()
        .lock()
        .map(|mut claims| claims.insert(job_id.to_string()))
        .unwrap_or(true)
}

fn release_fire_claim(job_id: &str) {
    if let Ok(mut claims) = fire_claims().lock() {
        claims.remove(job_id);
    }
}

/// Deliver one cron job's final response to its resolved targets (hermes
/// `_deliver_result`, live-adapter lane): wraps the content when
/// configured, strips `MEDIA:` tags (senders are text-only), and sends
/// via each platform's registered sender. Returns None on success /
/// nothing-to-deliver, or an error string.
async fn deliver_job_result(job: &CronJob, content: &str, wrap_response: bool) -> Option<String> {
    let targets = crate::cron::delivery::resolve_delivery_targets(job);
    if targets.is_empty() {
        let deliver = crate::cron::delivery::normalize_deliver_value(
            job.deliver
                .as_ref()
                .map(|deliver| Value::String(deliver.clone()))
                .as_ref(),
        );
        if deliver == "local" {
            return None; // local-only jobs don't deliver — not a failure
        }
        // deliver=origin with no resolvable origin and no configured
        // home channels: treat as local rather than reporting an error
        // (hermes #43014) — output stays in the run/session.
        if deliver == "origin" {
            return None;
        }
        return Some(format!(
            "no delivery target resolved for deliver={}",
            deliver
        ));
    }
    let body = if wrap_response {
        crate::cron::delivery::wrap_delivery_content(job, content)
    } else {
        content.to_string()
    };
    let (cleaned, _media) = crate::messaging::extract_media_tags(&body);
    let mut errors: Vec<String> = Vec::new();
    for target in targets {
        let key = crate::cron::delivery::sender_key_for(&target.platform);
        match crate::messaging::platform_sender(&key) {
            Some(sender) => {
                sender.send_text(&target.chat_id, &cleaned).await;
            }
            None => errors.push(format!(
                "platform '{}' has no registered sender",
                target.platform
            )),
        }
    }
    if errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    }
}

/// Dispatch one cron job as a tracked run (shared by `POST /api/jobs/:id/run`,
/// the scheduler, and the Chronos fire webhook): creates the cron-run
/// session + run row, records the outcome back onto the job when the run
/// finishes (incl. external delivery + `last_delivery_error`), and
/// executes the turn inside the cron approval scope. Returns the run id.
async fn spawn_job_run(
    state: Arc<GatewayState>,
    job: &mut CronJob,
    store: Arc<CronStore>,
) -> Option<String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let session_id = state
        .store
        .create_session("cron-run", Some(&state.model_name), None)
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let run = RunState {
        run_id: run_id.clone(),
        status: "running".to_string(),
        session_id: Some(session_id.clone()),
        message: job.prompt.clone(),
        created_at: now_secs(),
        finished_at: None,
        result: None,
        error: None,
        iterations: None,
        stop_requested: false,
        approval: None,
    };
    state.runs.lock().await.insert(run_id.clone(), run);
    job.last_run = Some(now_secs());
    job.last_status = Some("running".to_string());
    store.update(job).ok();

    // Record the outcome on the job row once the run finishes, then
    // deliver the final response to the configured target(s) (hermes
    // `run_one_job` delivery block).
    let job_store = Arc::clone(&store);
    let job_snapshot = job.clone();
    let job_id = job.id.clone();
    let runs = state.runs.clone();
    let outcome_run_id = run_id.clone();
    tokio::spawn(async move {
        let outcome = loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let snapshot = runs.lock().await.get(&outcome_run_id).map(|run| {
                (
                    run.status.clone(),
                    run.error.clone(),
                    run.result.clone(),
                )
            });
            match snapshot {
                None => break ("failed".to_string(), Some("run lost".to_string()), None),
                Some((status, error, result))
                    if matches!(status.as_str(), "completed" | "failed") =>
                {
                    break (status, error, result)
                }
                Some(_) => continue,
            }
        };
        let (status, mut run_error, result) = outcome;
        let mut success = status == "completed";
        // Empty final responses are a soft failure (hermes #8585): the
        // agent ran but produced nothing useful.
        let result_text = result.unwrap_or_default();
        if success && result_text.trim().is_empty() {
            success = false;
            run_error = Some(
                "Agent completed but produced empty response (model error, timeout, or misconfiguration)"
                    .to_string(),
            );
        }
        // Failed jobs deliver a compact failure summary; successful jobs
        // deliver the response unless it is a silence marker (hermes
        // `_is_cron_silence_response`).
        let wrap_response = crate::config::UlncLawConfig::load(None)
            .map(|config| config.cron.wrap_response)
            .unwrap_or(true);
        let mut delivery_error: Option<String> = None;
        if success {
            if !crate::cron::delivery::is_cron_silence_response(&result_text) {
                delivery_error =
                    deliver_job_result(&job_snapshot, &result_text, wrap_response).await;
            }
        } else {
            let summary = crate::cron::delivery::summarize_cron_failure_for_delivery(
                &job_snapshot,
                run_error.as_deref(),
            );
            if !summary.trim().is_empty() {
                delivery_error =
                    deliver_job_result(&job_snapshot, &summary, wrap_response).await;
            }
        }
        let delivery_failed = delivery_error.is_some();
        if let Ok(Some(mut job)) = job_store.get(&job_id) {
            job.last_status = Some(match (success, &run_error) {
                (true, _) => "ok".to_string(),
                (false, Some(error)) => format!("error: {}", error),
                (false, None) => format!("error: {}", status),
            });
            job.last_delivery_error = delivery_error;
            job_store.update(&job).ok();
        }
        release_fire_claim(&job_id);

        // Content-free cron execution lifecycle projection (hermes
        // CronExecutionEvent) — monitoring egress only.
        crate::monitoring::emit(
            crate::monitoring::CronExecutionEvent {
                status: if success { "ok".to_string() } else { "error".to_string() },
                job_key: job_id.clone(),
                source: job_snapshot
                    .origin
                    .as_ref()
                    .map(|o| format!("platform:{}", o.platform))
                    .unwrap_or_else(|| "cron".to_string()),
                duration_ms: None,
                delivery_outcome: Some(if delivery_failed {
                    "failed".to_string()
                } else {
                    "ok".to_string()
                }),
                error_class: run_error.as_ref().map(|_| "agent_error".to_string()),
                ts_ns: crate::monitoring::now_ns(),
            }
            .to_value(),
        );
    });

    spawn_tracked_run(state, run_id.clone(), session_id, job.prompt.clone(), true);
    Some(run_id)
}

/// Provider factory used by the dispatcher's auto-decompose tick (the
/// binary builds it from live config so `main.rs`' provider wiring is
/// reused without a library dependency on the CLI).
pub type DispatcherProviderFactory = std::sync::Arc<
    dyn Fn() -> std::result::Result<std::sync::Arc<dyn crate::provider::Provider>, String>
        + Send
        + Sync,
>;

/// Try to take the exclusive, non-blocking dispatcher lock at `path`
/// (hermes `_acquire_singleton_lock`). The returned handle must stay
/// open for as long as this process dispatches; dropping it releases
/// the lock.
fn try_acquire_dispatcher_lock_at(path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(file)
    } else {
        None
    }
}

/// Start the embedded kanban dispatcher loop (hermes hosts the dispatcher
/// in the gateway, ticking every 60 s by default): auto-decompose fresh
/// triage tasks, reclaim stale claims, promote parent-done todos, spawn
/// detached workers for ready tasks.
///
/// Only one gateway process machine-wide runs the dispatcher: an
/// exclusive advisory lock next to `kanban.db` is the backstop that
/// survives config drift and restart races (hermes singleton lock —
/// concurrent dispatchers double reclaim frequency and claim events).
///
/// The `[kanban] auto_decompose` toggle is re-read from config EVERY
/// tick (hermes #49638): it is a safety switch — flipping it off must
/// stop a runaway fan-out on the next tick, not on gateway restart. A
/// config read failure fails safe (no auto-decompose that tick).
pub fn spawn_kanban_dispatcher(
    interval_secs: u64,
    max_spawn: usize,
    use_worktrees: bool,
    provider_factory: Option<DispatcherProviderFactory>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let lock_path = crate::config::ulnclaw_home()
            .join("kanban")
            .join("dispatcher.lock");
        // Held for the loop's lifetime; dropping releases the lock.
        let _lock_guard = match try_acquire_dispatcher_lock_at(&lock_path) {
            Some(handle) => handle,
            None => {
                tracing::warn!(
                    "kanban dispatcher: another gateway holds {lock_path:?};                      this process will not dispatch (dispatch_in_gateway backstop)"
                );
                return;
            }
        };
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(5)));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Health telemetry (hermes HEALTH_WINDOW): warn when spawnable
        // ready work keeps waiting but nothing spawns, throttled to
        // 300 s. Claim-pulled lanes don't count (has_spawnable_ready).
        const HEALTH_WINDOW: u32 = 6;
        let mut bad_ticks: u32 = 0;
        let mut last_warn_at: i64 = 0;
        loop {
            interval.tick().await;
            auto_decompose_tick(provider_factory.as_ref()).await;
            let tick_outcome = tokio::task::spawn_blocking(move || -> Option<(usize, bool)> {
                let Ok(store) = crate::kanban::KanbanStore::open_default() else {
                    return None;
                };
                let home = crate::config::ulnclaw_home();
                // Live re-read so [kanban] stale_timeout_seconds edits
                // take effect without a gateway restart.
                let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
                let stale_timeout = config.kanban.stale_timeout_seconds;
                let known_profiles: std::collections::HashSet<String> =
                    config.profiles.keys().cloned().collect();
                match store.dispatch_once(
                    &home,
                    use_worktrees,
                    |task, workspace| {
                        crate::kanban::dispatch_spawn(&home, task, workspace)
                    },
                    Some(max_spawn.max(1)),
                    false,
                    2,
                    stale_timeout,
                    Some(&known_profiles),
                    config.kanban.max_in_progress_per_profile,
                    config.kanban.max_in_progress,
                ) {
                    Ok(result) => {
                        if !result.spawned.is_empty() || !result.reclaimed.is_empty() {
                            tracing::info!(
                                "kanban dispatch: {} reclaimed, {} promoted, {} spawned",
                                result.reclaimed.len(),
                                result.promoted.len(),
                                result.spawned.len()
                            );
                        }
                        // Hermes health probe: only count work the
                        // dispatcher would actually spawn (unknown
                        // assignees are claim-pulled lanes, correctly
                        // idle — not stuck).
                        let ready_pending = store
                            .has_spawnable_ready(Some(&known_profiles))
                            .unwrap_or(false)
                            || store
                                .has_spawnable_review(Some(&known_profiles))
                                .unwrap_or(false);
                        Some((result.spawned.len(), ready_pending))
                    }
                    Err(e) => {
                        tracing::warn!("kanban dispatch tick failed: {e}");
                        None
                    }
                }
            })
            .await;
            if let Ok(Some((spawned, ready_pending))) = tick_outcome {
                if ready_pending && spawned == 0 {
                    bad_ticks += 1;
                } else {
                    bad_ticks = 0;
                }
                if bad_ticks >= HEALTH_WINDOW {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    if now - last_warn_at >= 300 {
                        tracing::warn!(
                            "kanban dispatcher stuck: spawnable ready tasks waiting for                              {bad_ticks} consecutive ticks but 0 workers spawned.                              Check profile health (binary, PATH, credentials) and                              `ulnclaw kanban list --status ready`."
                        );
                        last_warn_at = now;
                    }
                }
            }
        }
    })
}

/// One auto-decompose pass (hermes `_auto_decompose_tick`): turn fresh
/// triage tasks into ready workgraphs before the dispatcher fans out
/// workers. Capped by `auto_decompose_per_tick` so a bulk load of triage
/// tasks does not burst-spend the auxiliary LLM in one tick; the
/// remainder defers to subsequent ticks.
async fn auto_decompose_tick(provider_factory: Option<&DispatcherProviderFactory>) {
    // Fail safe: a config read error disables auto-decompose for this
    // tick rather than falling back to burst-prone defaults (hermes
    // _resolve_auto_decompose_settings).
    let Ok(config) = crate::config::UlncLawConfig::load(None) else {
        return;
    };
    if !config.kanban.auto_decompose {
        return;
    }
    let Some(factory) = provider_factory else {
        return;
    };
    let provider = match factory() {
        Ok(provider) => provider,
        Err(e) => {
            tracing::debug!("kanban auto-decompose: provider unavailable ({e})");
            return;
        }
    };
    let Ok(store) = crate::kanban::KanbanStore::open_default() else {
        return;
    };
    let triage_ids = match crate::kanban_triage::list_triage_ids(&store) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::debug!("kanban auto-decompose: list_triage_ids failed ({e})");
            return;
        }
    };
    let per_tick = config.kanban.auto_decompose_per_tick.max(1);
    for task_id in triage_ids.into_iter().take(per_tick) {
        let outcome = crate::kanban_triage::decompose_task(
            &store,
            &config,
            provider.clone(),
            &task_id,
            Some("auto-decomposer"),
        )
        .await;
        if outcome.ok {
            match &outcome.child_ids {
                Some(children) if outcome.fanout => {
                    tracing::info!(
                        "kanban auto-decompose: {task_id} → {} children",
                        children.len()
                    );
                }
                _ => {
                    tracing::info!("kanban auto-decompose: {task_id} → single task (no fanout)");
                }
            }
        } else {
            // Common no-op reasons (no aux client configured) must not
            // spam logs every tick (hermes logs them at debug).
            tracing::debug!("kanban auto-decompose: {task_id} skipped: {}", outcome.reason);
        }
    }
}

/// Terminal event kinds delivered to notify subscribers (hermes
/// `TERMINAL_KINDS`). `archived` / `unblocked` are claimed (cursor
/// advances past them so they can't wedge a later completed/blocked
/// event behind an unclaimed row) but intentionally silent.
const NOTIFY_TERMINAL_KINDS: &[&str] = &[
    "completed",
    "blocked",
    "gave_up",
    "crashed",
    "timed_out",
    "status",
    "archived",
    "unblocked",
];

/// Start the kanban notification delivery loop (hermes kanban notifier):
/// every 5 s, poll `kanban_notify_subs`, deliver unseen terminal events
/// to each subscribed chat via the registered platform sender, advance
/// the subscription cursor after delivery, and drop the subscription
/// once the task reaches a truly final status (done / archived).
/// Gateway chat entry point targeted by the kanban wake self-post
/// (hermes wake routing needs the in-process API server's bind + key).
#[derive(Debug, Clone)]
pub struct WakeEndpoint {
    pub host: String,
    pub port: u16,
    pub key: Option<String>,
}

/// A wake self-post runs an entire agent turn synchronously
/// (stream=false); generous ceiling so long tool-using turns are not
/// killed mid-flight (hermes `WAKE_TURN_TIMEOUT_SECONDS`).
const WAKE_TURN_TIMEOUT_SECONDS: u64 = 600;

/// Backoff between retries on transient failures (429 concurrency cap,
/// connection errors) — hermes `_RETRY_DELAYS_SECONDS`.
const WAKE_RETRY_DELAYS_SECONDS: [u64; 3] = [2, 5, 10];

/// Wake the creator session by self-POSTing the wake text to the
/// gateway's own `/v1/chat/completions` with the raw session id in
/// `X-Ulnclaw-Session-Id` — the exact entry point real turns use, so
/// the wake resumes the REAL session with full history (hermes
/// `_self_post_chat_completion`). Errors are returned so the caller
/// logs instead of silently losing the event.
pub async fn deliver_session_wake(
    endpoint: &WakeEndpoint,
    session_id: &str,
    text: &str,
) -> std::result::Result<(), String> {
    let mut host = endpoint.host.clone();
    if host == "0.0.0.0" || host == "::" || host == "*" {
        // Wildcard bind — connect over loopback.
        host = "127.0.0.1".to_string();
    }
    if host.contains(':') && !host.starts_with('[') {
        host = format!("[{host}]"); // bare IPv6 literal
    }
    let url = format!("http://{host}:{}/v1/chat/completions", endpoint.port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(WAKE_TURN_TIMEOUT_SECONDS))
        .build()
        .map_err(|e| format!("wake http client: {e}"))?;
    let mut last_err = String::new();
    let attempts = 1 + WAKE_RETRY_DELAYS_SECONDS.len();
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(
                WAKE_RETRY_DELAYS_SECONDS[attempt - 1],
            ))
            .await;
        }
        let mut request = client
            .post(&url)
            .header("X-Ulnclaw-Session-Id", session_id)
            .json(&serde_json::json!({
                "model": "ulnclaw",
                "messages": [{ "role": "user", "content": text }],
                "stream": false
            }));
        if let Some(key) = endpoint.key.as_deref().filter(|k| !k.is_empty()) {
            request = request.bearer_auth(key);
        }
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    last_err = format!(
                        "wake self-post got HTTP 429 (concurrency cap) \
                         for session {session_id}"
                    );
                    tracing::warn!(
                        "{last_err}; attempt {}/{}",
                        attempt + 1,
                        attempts
                    );
                    continue;
                }
                if status.is_client_error() || status.is_server_error() {
                    let body = response.text().await.unwrap_or_default();
                    let body: String = body.chars().take(300).collect();
                    return Err(format!(
                        "wake self-post failed for session {session_id}: \
                         HTTP {status}: {body}"
                    ));
                }
                tracing::info!(
                    "kanban notifier: wake delivered for session {session_id} \
                     (attempt {})",
                    attempt + 1
                );
                return Ok(());
            }
            Err(e) => {
                last_err = format!(
                    "wake self-post transient failure for session {session_id}: {e}"
                );
                tracing::warn!(
                    "{last_err} (attempt {}/{})",
                    attempt + 1,
                    attempts
                );
            }
        }
    }
    Err(format!(
        "wake self-post gave up for session {session_id} after \
         {attempts} attempts: {last_err}"
    ))
}

pub fn spawn_kanban_notifier(
    wake: Option<WakeEndpoint>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Initial delay so the gateway can finish wiring platform
        // adapters (hermes does the same 5 s wait).
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            notifier_tick(wake.as_ref()).await;
        }
    })
}

/// One notifier pass over every subscription.
async fn notifier_tick(wake: Option<&WakeEndpoint>) {
    let Ok(store) = crate::kanban::KanbanStore::open_default() else {
        return;
    };
    let subs = match store.list_notify_subs(None) {
        Ok(subs) => subs,
        Err(e) => {
            tracing::debug!("kanban notifier: list_notify_subs failed ({e})");
            return;
        }
    };
    for sub in subs {
        // No connected adapter for this platform: skip WITHOUT advancing
        // the cursor so events are still delivered once the adapter
        // comes up (hermes rewind semantics).
        let Some(sender) = crate::messaging::platform_sender(&sub.platform) else {
            continue;
        };
        let thread = if sub.thread_id.is_empty() {
            None
        } else {
            Some(sub.thread_id.as_str())
        };
        let (cursor, events) = match store.unseen_events_for_sub(
            &sub.task_id,
            &sub.platform,
            &sub.chat_id,
            thread,
            Some(NOTIFY_TERMINAL_KINDS),
        ) {
            Ok(page) => page,
            Err(e) => {
                tracing::debug!("kanban notifier: unseen events for {} failed ({e})", sub.task_id);
                continue;
            }
        };
        if events.is_empty() {
            continue;
        }
        let task = store.get_task(&sub.task_id).ok().flatten();
        for event in &events {
            let Some(message) = format_notify_message(task.as_ref(), event) else {
                continue; // silent kinds (archived / unblocked)
            };
            // Scoped delivery: PlatformSender has no thread routing or
            // failure channel, so thread_id rides in the subscription
            // row only and sends are assumed delivered.
            sender.send_text(&sub.chat_id, &message).await;
            tracing::debug!(
                "kanban notifier: delivered {} event for {} to {}/{}",
                event.kind,
                sub.task_id,
                sub.platform,
                sub.chat_id
            );
        }
        if let Err(e) = store.advance_notify_cursor(
            &sub.task_id,
            &sub.platform,
            &sub.chat_id,
            thread,
            cursor,
        ) {
            tracing::debug!("kanban notifier: cursor advance failed ({e})");
        }
        // Wake routing (hermes): terminal events ALSO resume the
        // creator session recorded on the task. The text ping above was
        // the delivery, so the wake is best-effort and runs detached —
        // a slow agent turn must not stall other subscriptions.
        if let Some(endpoint) = wake {
            if let Some(task) = task.as_ref() {
                if let Some(session_id) = task
                    .session_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                {
                    let present: std::collections::HashSet<&str> = events
                        .iter()
                        .map(|e| e.kind.as_str())
                        .filter(|kind| crate::kanban::WAKE_KINDS.contains(kind))
                        .collect();
                    // Fixed hermes order: completed, gave_up, crashed,
                    // timed_out, blocked.
                    let wake_kinds: Vec<String> = crate::kanban::WAKE_KINDS
                        .iter()
                        .filter(|kind| present.contains(*kind))
                        .map(|kind| kind.to_string())
                        .collect();
                    if !wake_kinds.is_empty() {
                        let text = crate::kanban::wake_message(
                            task,
                            &wake_kinds,
                            &task.board,
                        );
                        let session_id = session_id.to_string();
                        let endpoint = endpoint.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                deliver_session_wake(&endpoint, &session_id, &text)
                                    .await
                            {
                                tracing::warn!(
                                    "kanban notifier: wakeup failed for \
                                     {session_id}: {e}"
                                );
                            }
                        });
                    }
                }
            }
        }
        // Subscriptions are removed only when the task reaches a truly
        // final status (done / archived) — crash/retry cycles must keep
        // notifying (hermes policy; the cursor handles dedup).
        if let Some(task) = task.as_ref() {
            if task.status == "done" || task.status == "archived" {
                store
                    .remove_notify_sub(&sub.task_id, &sub.platform, &sub.chat_id, thread)
                    .ok();
            }
        }
    }
}

/// Render one terminal event as a chat message (hermes notifier message
/// formats). Returns `None` for intentionally silent kinds.
fn format_notify_message(
    task: Option<&crate::kanban::Task>,
    event: &crate::kanban::TaskEvent,
) -> Option<String> {
    let board_tag = task
        .map(|t| format!("[{}] ", t.board))
        .unwrap_or_default();
    let who_tag = task
        .and_then(|t| t.assignee.as_deref())
        .filter(|a| !a.is_empty())
        .map(|a| format!("@{a} "))
        .unwrap_or_default();
    let task_id = task.map(|t| t.id.as_str()).unwrap_or("?");
    let title: String = task
        .map(|t| t.title.chars().take(120).collect())
        .unwrap_or_default();
    let payload_str = |key: &str, limit: usize| -> Option<String> {
        let s = event.payload.get(key)?.as_str()?;
        let first = s.trim().lines().next().unwrap_or("");
        Some(first.chars().take(limit).collect())
    };
    let message = match event.kind.as_str() {
        "completed" => {
            let handoff = payload_str("summary", 200)
                .or_else(|| payload_str("result", 200))
                .or_else(|| {
                    task.and_then(|t| t.result.as_deref())
                        .map(|r| r.trim().lines().next().unwrap_or("").chars().take(160).collect())
                })
                .map(|h| format!("\n{h}"))
                .unwrap_or_default();
            format!("✔ {board_tag}{who_tag}Kanban {task_id} done — {title}{handoff}")
        }
        "blocked" => {
            let reason = payload_str("reason", 160)
                .map(|r| format!(": {r}"))
                .unwrap_or_default();
            format!("⏸ {board_tag}{who_tag}Kanban {task_id} blocked{reason}")
        }
        "gave_up" => {
            let err = payload_str("error", 200)
                .map(|e| format!("\n{e}"))
                .unwrap_or_default();
            format!("✖ {board_tag}{who_tag}Kanban {task_id} gave up after repeated spawn failures{err}")
        }
        "crashed" => format!(
            "✖ {board_tag}{who_tag}Kanban {task_id} worker crashed (pid gone); dispatcher will retry"
        ),
        "timed_out" => {
            let limit = event
                .payload
                .get("limit_seconds")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            format!(
                "⏱ {board_tag}{who_tag}Kanban {task_id} timed out (max_runtime={limit}s); will retry"
            )
        }
        "status" => {
            let new_status = payload_str("status", 40).unwrap_or_default();
            format!("🔄 {board_tag}{who_tag}Kanban {task_id} → {new_status}")
        }
        // archived / unblocked advance the cursor silently.
        _ => return None,
    };
    Some(message)
}

/// Start the cron scheduler loop (hermes scheduler): every `poll_secs`
/// dispatch each due job as a tracked cron run. Called from the gateway
/// command once the cron store is wired; does nothing when absent.
/// Start the monitoring plane (hermes `agent/monitoring` gateway wiring):
/// the OTLP streamer consuming the emitter queue, plus a periodic
/// content-free health sampler. No-op unless
/// `[monitoring] gateway_health_export.enabled` AND `export.otlp.enabled`
/// with an endpoint. Never affects gateway operation (fail-isolated).
pub fn spawn_monitoring(
    state: Arc<GatewayState>,
    config: &crate::config::UlncLawConfig,
    platform_count: u64,
) -> Vec<tokio::task::JoinHandle<()>> {
    let monitoring = &config.monitoring;
    if !monitoring.enabled() || !monitoring.otlp_enabled() {
        return Vec::new();
    }
    let Some(endpoint) = monitoring
        .export
        .otlp
        .endpoint
        .clone()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
    else {
        tracing::warn!("monitoring.gateway_health_export enabled but monitoring.export.otlp.endpoint is not set");
        return Vec::new();
    };

    let install_id = crate::monitoring::ensure_install_id(monitoring.install_id.as_deref());
    let version = env!("CARGO_PKG_VERSION");
    let resource = crate::monitoring::resource_attributes(&install_id, version, None);

    let mut handles = Vec::new();

    // Lifecycle event (hermes gateway_started).
    crate::monitoring::emit(
        crate::monitoring::GatewayHealthEvent {
            name: "gateway_started".to_string(),
            gateway_state: Some("running".to_string()),
            old_state: None,
            new_state: Some("running".to_string()),
            exit_reason: None,
            active_agents: 0,
            gateway_busy: false,
            platform_count,
            profile: None,
            install_id: Some(install_id.clone()),
            version: Some(version.to_string()),
            pid: Some(std::process::id()),
            ts_ns: crate::monitoring::now_ns(),
        }
        .to_value(),
    );

    // Continuous streamer (hermes OTLPStreamer).
    if monitoring.diagnostic_events_enabled() || monitoring.metrics_enabled() {
        let streamer_endpoint = endpoint.clone();
        let headers_env = monitoring.export.otlp.headers_env.clone();
        let streamer_resource = resource.clone();
        let flush = std::time::Duration::from_secs(monitoring.logs_export_interval_seconds());
        handles.push(tokio::spawn(async move {
            crate::monitoring::run_streamer(streamer_endpoint, headers_env, streamer_resource, flush)
                .await;
        }));
    }

    // Periodic health snapshot (hermes gateway_health_export metrics loop).
    if monitoring.metrics_enabled() {
        let interval = std::time::Duration::from_secs(monitoring.export_interval_seconds());
        let sampler_install_id = install_id.clone();
        let sampler_version = version.to_string();
        handles.push(tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let active = state.runs.lock().await.len() as u64;
                crate::monitoring::emit(
                    crate::monitoring::GatewayHealthEvent {
                        name: "heartbeat".to_string(),
                        gateway_state: Some("running".to_string()),
                        old_state: None,
                        new_state: None,
                        exit_reason: None,
                        active_agents: active,
                        gateway_busy: active > 0,
                        platform_count,
                        profile: None,
                        install_id: Some(sampler_install_id.clone()),
                        version: Some(sampler_version.clone()),
                        pid: Some(std::process::id()),
                        ts_ns: crate::monitoring::now_ns(),
                    }
                    .to_value(),
                );
            }
        }));
    }

    tracing::info!(
        "monitoring enabled: OTLP export to {} (install_id {})",
        crate::monitoring::traces_url(&endpoint),
        install_id
    );
    handles
}

pub fn spawn_cron_scheduler(state: Arc<GatewayState>, poll_secs: u64) -> Option<tokio::task::JoinHandle<()>> {
    let store = state.cron.get().cloned()?;
    Some(tokio::spawn(crate::cron::run_scheduler(
        store,
        poll_secs,
        move |job| {
            let state = state.clone();
            let run = async move {
                if job.prompt.trim().is_empty() {
                    return Err(crate::error::AgentError::config("job has no prompt to run"));
                }
                let store = match state.cron.get().cloned() {
                    Some(store) => store,
                    None => return Err(crate::error::AgentError::config("cron store unavailable")),
                };
                let mut job = job;
                match spawn_job_run(state, &mut job, store).await {
                    Some(run_id) => Ok(format!("running (run {})", run_id)),
                    None => Err(crate::error::AgentError::config("failed to start job run")),
                }
            };
            // Hermes parity: the scheduler installs a profile secret scope
            // around every job run, so scoped credential reads inside the
            // job resolve against the gateway home's `.env` overlay.
            let home = crate::config::ulnclaw_home();
            crate::secret_scope::scope_secrets(
                std::sync::Arc::new(crate::secret_scope::build_profile_secret_scope(&home)),
                run,
            )
        },
    )))
}

// ---------------------------------------------------------------------------
// Discovery: skills + toolsets
// ---------------------------------------------------------------------------

/// `GET /v1/skills` — list installed skills (hermes `_handle_skills`).
async fn skills_list(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let dir = state
        .skills_dir
        .get()
        .cloned()
        .or_else(|| crate::config::ensure_home().ok().map(|home| home.join("skills")));
    let skills = dir
        .map(|dir| crate::skills::list_skills(&dir))
        .unwrap_or_default();
    Json(json!({"object": "list", "data": skills}))
}

/// `GET /v1/toolsets` — list toolsets and their resolved tools
/// (hermes `_handle_toolsets`). A toolset counts as enabled when the agent
/// actually exposes at least one of its tools.
async fn toolsets_list(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let registered: std::collections::HashSet<String> =
        state.agent.tool_names().into_iter().collect();
    let definitions = crate::toolsets::toolsets();
    let mut names: Vec<&str> = definitions.keys().copied().collect();
    names.sort_unstable();
    let data: Vec<Value> = names
        .into_iter()
        .map(|name| {
            let definition = &definitions[name];
            let mut tools = crate::toolsets::resolve_toolset(name);
            tools.sort();
            tools.dedup();
            let enabled = tools.iter().any(|tool| registered.contains(tool));
            json!({
                "name": name,
                "description": definition.description,
                "enabled": enabled,
                "tools": tools,
            })
        })
        .collect();
    Json(json!({"object": "list", "data": data}))
}

// ---------------------------------------------------------------------------
// Async runs API
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StartRunRequest {
    message: String,
    #[serde(default)]
    session_id: Option<String>,
}

async fn start_run(
    State(state): State<Arc<GatewayState>>,
    Json(request): Json<StartRunRequest>,
) -> Response {
    if request.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "message is required", "type": "invalid_request_error"}})),
        )
            .into_response();
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    // Runs always own a session so approvals can be session-scoped and the
    // conversation stays continuable.
    let session_id = match request.session_id.clone() {
        Some(sid) if !sid.trim().is_empty() => sid,
        _ => state
            .store
            .create_session("gateway-run", Some(&state.model_name), None)
            .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string()),
    };
    let run = RunState {
        run_id: run_id.clone(),
        status: "running".to_string(),
        session_id: Some(session_id.clone()),
        message: request.message.clone(),
        created_at: now_secs(),
        finished_at: None,
        result: None,
        error: None,
        iterations: None,
        stop_requested: false,
        approval: None,
    };
    state.runs.lock().await.insert(run_id.clone(), run.clone());

    spawn_tracked_run(state, run_id.clone(), session_id.clone(), request.message.clone(), false);

    (StatusCode::ACCEPTED, Json(json!({"run_id": run_id, "status": "running", "session_id": session_id}))).into_response()
}

/// Execute one agent turn as a tracked background run: registers approval
/// routing, pumps pending approvals into run state, and updates the run row
/// when the turn finishes. Shared by `/v1/runs` (`cron = false`) and
/// `/api/jobs/:id/run` (`cron = true` — approval gates apply
/// `approvals.cron_mode` because no human is attached).
fn spawn_tracked_run(
    state: Arc<GatewayState>,
    run_id: String,
    session_id: String,
    message: String,
    cron: bool,
) {
    // Approval plumbing: channel from the approve callback into run state.
    let (approval_tx, mut approval_rx) = tokio::sync::mpsc::unbounded_channel::<PendingApproval>();
    state.router.register(&run_id, &session_id, approval_tx);
    let pump = state.clone();
    let pump_run_id = run_id.clone();
    tokio::spawn(async move {
        while let Some(pending) = approval_rx.recv().await {
            {
                let mut runs = pump.runs.lock().await;
                if let Some(run) = runs.get_mut(&pump_run_id) {
                    run.status = "waiting_for_approval".to_string();
                    run.approval = Some(json!({
                        "command": pending.command,
                        "reason": pending.reason,
                        "choices": ["once", "session", "always", "deny"],
                    }));
                }
            }
            pump.pending_approvals
                .lock()
                .await
                .insert(pump_run_id.clone(), pending.respond);
        }
    });

    state
        .metrics
        .runs_started
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let runner = state.clone();
    let spawn_run_id = run_id.clone();
    let run_future = RUN_ID.scope(
        run_id.clone(),
        crate::agent::cron_scope(cron, async move {
        let history = runner
            .store
            .load_messages(&session_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role != Role::System)
            .collect::<Vec<_>>();
        let history = if history.is_empty() { None } else { Some(history) };
        let override_model = session_model_override(&runner, &session_id);
        let outcome = await_with_model_override(
            override_model,
            runner.agent.run_with_session(&message, history, Some(session_id.as_str())),
        )
        .await;
        let mut runs = runner.runs.lock().await;
        if let Some(run) = runs.get_mut(&spawn_run_id) {
            match outcome {
                Ok(result) => {
                    run.status = "completed".to_string();
                    runner.metrics.record_run(&result.usage, result.tool_calls.len());
                    runner
                        .metrics
                        .runs_completed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    run.result = Some(result.content);
                    run.session_id = result.session_id.or(run.session_id.take());
                    run.iterations = Some(result.iterations);
                }
                Err(e) => {
                    run.status = "failed".to_string();
                    runner
                        .metrics
                        .runs_failed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    run.error = Some(e.to_string());
                }
            }
            run.finished_at = Some(now_secs());
        }
        drop(runs);
        runner.router.unregister(&spawn_run_id);
        }),
    );
    // Profile secret scope inheritance (hermes copy_context parity): a
    // mirrored `/p/<profile>/...` request dispatches with that profile's
    // scope installed; the run task must keep resolving credentials
    // against it, so re-install the captured scope inside the spawn.
    crate::secret_scope::spawn_scoped(run_future);
}

async fn list_runs(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    let runs = state.runs.lock().await;
    let mut data: Vec<&RunState> = runs.values().collect();
    data.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
    Json(json!({"object": "list", "data": data}))
}

async fn get_run(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let runs = state.runs.lock().await;
    match runs.get(&id) {
        Some(run) => Json(json!(run)).into_response(),
        None => not_found(&format!("run {} not found", id)),
    }
}

/// SSE stream of run lifecycle events (`run.progress` / `run.completed` /
/// `run.failed`), closing once the run reaches a terminal state.
async fn run_events(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    let exists = state.runs.lock().await.contains_key(&id);
    if !exists {
        return not_found(&format!("run {} not found", id));
    }
    let stream = futures::stream::unfold(
        (state, id, String::new(), false),
        |(state, id, last_status, done)| async move {
            if done {
                return None;
            }
            loop {
                let run = state.runs.lock().await.get(&id).cloned();
                match run {
                    None => return None,
                    Some(run) if run.status != last_status => {
                        let event_name = match run.status.as_str() {
                            "completed" => "run.completed",
                            "failed" => "run.failed",
                            "waiting_for_approval" => "approval.request",
                            _ => "run.progress",
                        };
                        let data = serde_json::to_string(&run).unwrap_or_default();
                        let event = axum::response::sse::Event::default()
                            .event(event_name)
                            .data(data);
                        let terminal = matches!(run.status.as_str(), "completed" | "failed");
                        return Some((
                            Ok::<_, std::convert::Infallible>(event),
                            (state, id, run.status.clone(), terminal),
                        ));
                    }
                    Some(_) => {
                        drop(run);
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                }
            }
        },
    );
    axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

#[derive(Deserialize)]
struct ApprovalDecision {
    decision: String,
}

/// Resolve a run's pending approval: once | session | always | deny.
async fn resolve_approval(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(request): Json<ApprovalDecision>,
) -> Response {
    let decision = request.decision.trim().to_string();
    if !matches!(decision.as_str(), "once" | "session" | "always" | "deny") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "decision must be one of: once, session, always, deny", "type": "invalid_request_error"}})),
        )
            .into_response();
    }
    let responder = state.pending_approvals.lock().await.remove(&id);
    let Some(responder) = responder else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": {"message": format!("run {} has no pending approval", id), "type": "invalid_request_error"}})),
        )
            .into_response();
    };
    responder.send(decision.clone()).ok();
    let mut runs = state.runs.lock().await;
    if let Some(run) = runs.get_mut(&id) {
        if run.status == "waiting_for_approval" {
            run.status = "running".to_string();
        }
        if let Some(approval) = run.approval.as_mut() {
            approval["resolved"] = json!(decision);
        }
    }
    Json(json!({"run_id": id, "decision": decision})).into_response()
}

async fn stop_run(State(state): State<Arc<GatewayState>>, Path(id): Path<String>) -> Response {
    let mut runs = state.runs.lock().await;
    match runs.get_mut(&id) {
        Some(run) if run.status == "running" || run.status == "queued" => {
            run.stop_requested = true;
            Json(json!({
                "run_id": id,
                "stop_requested": true,
                "note": "interruption is best-effort in this build; the agent finishes its current iteration",
            }))
            .into_response()
        }
        Some(run) => Json(json!({
            "run_id": id,
            "stop_requested": false,
            "note": format!("run already {}", run.status),
        }))
        .into_response(),
        None => not_found(&format!("run {} not found", id)),
    }
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn server_error(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": {"message": message, "type": "server_error"}})),
    )
        .into_response()
}

fn not_found(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": {"message": message, "type": "not_found"}})),
    )
        .into_response()
}

fn bad_request(message: &str, code: Option<&str>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// MCP OAuth dashboard bridge (hermes web_routers/mcp.py `auth_mcp_server`
// / `mcp_oauth_flow_status` / `mcp_oauth_callback` + web_server.py
// `_run_dashboard_mcp_oauth`)
// ---------------------------------------------------------------------------

/// Build the externally reachable callback URL for a dashboard flow
/// (hermes `_mcp_oauth_callback_url`): scheme from X-Forwarded-Proto,
/// host from the Host header, percent-encoded server name.
fn mcp_oauth_callback_url(headers: &HeaderMap, server_name: &str) -> String {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:8642");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("http");
    let encoded: String =
        url::form_urlencoded::byte_serialize(server_name.as_bytes()).collect();
    format!("{scheme}://{host}/api/mcp/oauth/callback/{encoded}")
}

/// `POST /api/mcp/servers/:name/auth` — start MCP OAuth and hand the
/// authorization URL to the dashboard browser (hermes `auth_mcp_server`).
async fn mcp_server_auth(
    State(_state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    use crate::mcp::dashboard_oauth::{self as bridge, DashboardOAuthFlow, RegistryError};

    bridge::registry().gc();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let Some(server) = config.mcp.servers.iter().find(|srv| srv.name == name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("Server '{}' not found", name)})),
        )
            .into_response();
    };
    if server.url.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "stdio servers authenticate via env keys, not OAuth"})),
        )
            .into_response();
    }
    if !server.headers.is_empty() && server.auth.as_deref() != Some("oauth") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "This server uses header/API-key auth, not OAuth"})),
        )
            .into_response();
    }

    let home = crate::config::ulnclaw_home();
    let redirect_uri = server
        .oauth
        .redirect_uri
        .clone()
        .unwrap_or_else(|| mcp_oauth_callback_url(&headers, &name));
    let flow = DashboardOAuthFlow::new(
        uuid::Uuid::new_v4().simple().to_string(),
        name.clone(),
        None,
        home.to_string_lossy().to_string(),
        redirect_uri,
    );
    if let Err(err) = bridge::registry().insert(flow.clone()) {
        let (status, detail) = match err {
            RegistryError::TooManyPending => (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many MCP OAuth flows are already in progress".to_string(),
            ),
            RegistryError::AlreadyInProgress => (
                StatusCode::CONFLICT,
                format!("MCP OAuth for '{}' is already in progress", name),
            ),
        };
        return (status, Json(json!({"detail": detail}))).into_response();
    }

    tokio::spawn(run_dashboard_mcp_oauth(flow.clone(), server));

    if let Err(exc) = flow
        .wait_for_authorization_url(std::time::Duration::from_secs(30))
        .await
    {
        flow.mark_error(&exc.to_string());
    }
    Json(flow.snapshot()).into_response()
}

/// `GET /api/mcp/oauth/flows/:flow_id` — poll flow status + discovered
/// tools (hermes `mcp_oauth_flow_status`).
async fn mcp_oauth_flow_status(Path(flow_id): Path<String>) -> Response {
    use crate::mcp::dashboard_oauth as bridge;

    bridge::registry().gc();
    let Some(flow) = bridge::registry().get(&flow_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "OAuth flow not found or expired"})),
        )
            .into_response();
    };
    Json(flow.snapshot_with_tools()).into_response()
}

#[derive(Deserialize)]
struct McpOAuthCallbackParams {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// `GET /api/mcp/oauth/callback/:server_name` — the OAuth provider
/// redirect target (hermes `mcp_oauth_callback`). Open route: the
/// browser carries no bearer token; the `state` parameter authenticates
/// the delivery.
async fn mcp_oauth_callback(
    Path(server_name): Path<String>,
    Query(params): Query<McpOAuthCallbackParams>,
) -> Response {
    use crate::mcp::dashboard_oauth as bridge;

    bridge::registry().gc();
    let server_name = bridge::percent_decode(&server_name);
    let Some(flow) = bridge::registry().find_for_callback(&server_name, params.state.as_deref())
    else {
        return (
            StatusCode::NOT_FOUND,
            axum::response::Html(
                "<h1>OAuth flow expired</h1><p>Return to ulnclaw and try again.</p>",
            ),
        )
            .into_response();
    };
    if let Err(exc) = flow.deliver_callback(
        params.code.as_deref(),
        params.state.as_deref(),
        params.error.as_deref(),
    ) {
        let status = if exc.to_string().contains("already received") {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        };
        return (
            status,
            axum::response::Html(
                "<h1>OAuth callback rejected</h1><p>The callback was invalid or already used.</p>",
            ),
        )
            .into_response();
    }
    if params.error.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            axum::response::Html(
                "<h1>Authorization failed</h1><p>Return to ulnclaw for details.</p>",
            ),
        )
            .into_response();
    }
    axum::response::Html(
        "<h1>Authorization received</h1><p>You can close this tab and return to ulnclaw.</p>",
    )
    .into_response()
}

/// Dashboard OAuth worker (hermes `_run_dashboard_mcp_oauth`): force a
/// fresh authorization (tokens backed up first, restored on failure),
/// run the normal OAuth flow with the task-local flow installed, then
/// probe the authorized server for its tool list.
///
/// Divergence: hermes additionally reconnects the server into the live
/// agent registry; the ulnclaw gateway is a standalone daemon without a
/// tool registry, so new tokens take effect on the next session / lazy
/// spawn.
async fn run_dashboard_mcp_oauth(
    flow: Arc<crate::mcp::dashboard_oauth::DashboardOAuthFlow>,
    server: crate::mcp::McpServerConfig,
) {
    use crate::mcp::dashboard_oauth as bridge;

    let url = server.url.clone().unwrap_or_default();
    let home = crate::config::ulnclaw_home();
    let backup = crate::mcp::oauth::load_tokens(&home, &server.name);
    let result: Result<()> = bridge::scope_flow(flow.clone(), async {
        // Force a fresh authorization like hermes `manager.remove` ahead
        // of the probe; the backup is restored if the flow fails.
        crate::mcp::oauth::remove_tokens(&home, &server.name);
        let token =
            crate::mcp::oauth::get_access_token(&home, &server.name, &url, &server.oauth, true)
                .await?;
        // Post-auth probe: capture the tool list the dashboard shows
        // next to the approved flow (hermes `_probe_single_server`).
        let client = crate::mcp::remote::RemoteMcpClient::connect(
            &url,
            server.transport.as_deref(),
            &server.headers,
            Some(token),
            None,
            None,
        )
        .await?;
        let tools = client.list_tools().await?;
        let slim: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.get("name").cloned().unwrap_or(Value::Null),
                    "description": tool.get("description").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
        flow.set_tools(slim);
        flow.mark_approved()?;
        Ok(())
    })
    .await;
    if let Err(exc) = result {
        if let Some(tokens) = backup {
            crate::mcp::oauth::save_tokens(&home, &server.name, &tokens).ok();
        }
        flow.mark_error(&exc.to_string());
    }
    flow.mark_worker_done();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::openai::OpenAiProvider;
    use crate::tools::ToolRegistry;
    use tower::ServiceExt;

    fn test_state() -> Arc<GatewayState> {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Agent::new(provider, ToolRegistry::new()).with_store(store);
        GatewayState::new(Arc::new(agent), "test-model".into(), "test".into(), Some("sekret".into()), ApprovalRouter::new())
            .expect("state builds")
    }

    async fn get_json(app: Router, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = axum::http::Request::builder().uri(uri).method("GET");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {}", token));
        }
        let response = app
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn test_health_is_open_and_reports_ok() {
        let app = router(test_state());
        let (status, body) = get_json(app, "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], "ulnclaw-gateway");
    }

    fn learning_state(home: &std::path::Path) -> Arc<GatewayState> {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Agent::new(provider, ToolRegistry::new())
            .with_store(store)
            .with_context(crate::tools::context::ToolContext::default().with_home(home));
        GatewayState::new(Arc::new(agent), "test-model".into(), "test".into(), Some("sekret".into()), ApprovalRouter::new())
            .expect("state builds")
    }

    async fn send_json(
        app: Router,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = axum::http::Request::builder().uri(uri).method(method);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {}", token));
        }
        let request = request
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    fn seed_learning_home() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let skill_dir = home.join("skills").join("debug-helper");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: debug-helper\ncategory: debugging\n---\n\n# Debug helper\n",
        )
        .unwrap();
        std::fs::write(
            home.join("skills").join(".usage.json"),
            r#"{"debug-helper": {"use_count": 3, "created_by": "agent", "state": "active"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join("memory")).unwrap();
        std::fs::write(
            home.join("memory").join("MEMORY.md"),
            "# Memory\n\n- user prefers dark mode\n- deploy on fridays\n",
        )
        .unwrap();
        temp
    }

    #[tokio::test]
    async fn test_learning_graph_and_node_mutations() {
        let temp = seed_learning_home();
        let app = router(learning_state(temp.path()));

        // Graph: learned skill + both memory entries surface.
        let (status, body) = get_json(app.clone(), "/api/learning/graph", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<&str> = body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"debug-helper"));
        assert!(ids.contains(&"memory:memory:0"));
        assert!(ids.contains(&"memory:memory:1"));
        assert_eq!(body["stats"]["learned_skills"], 1);
        assert_eq!(body["stats"]["memory_nodes"], 2);

        // Node detail: skill + memory prefills.
        let (status, body) = get_json(
            app.clone(),
            "/api/learning/node?id=debug-helper",
            Some("sekret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["kind"], "skill");
        assert!(body["content"].as_str().unwrap().contains("Debug helper"));
        let (status, body) = get_json(
            app.clone(),
            "/api/learning/node?id=memory:memory:0",
            Some("sekret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["content"], "user prefers dark mode");
        let (status, _) = get_json(app.clone(), "/api/learning/node?id=nope", Some("sekret")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Edit a memory entry.
        let (status, body) = send_json(
            app.clone(),
            "PUT",
            "/api/learning/node",
            Some("sekret"),
            json!({"id": "memory:memory:0", "content": "user prefers light mode"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (_, body) = get_json(
            app.clone(),
            "/api/learning/node?id=memory:memory:0",
            Some("sekret"),
        )
        .await;
        assert_eq!(body["content"], "user prefers light mode");

        // Delete the second entry.
        let (status, body) = send_json(
            app.clone(),
            "DELETE",
            "/api/learning/node",
            Some("sekret"),
            json!({"id": "memory:memory:1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (_, body) = get_json(app.clone(), "/api/learning/graph", Some("sekret")).await;
        assert_eq!(body["stats"]["memory_nodes"], 1);
        drop(temp);
    }

    #[tokio::test]
    async fn test_auth_gates_api_routes() {
        let app = router(test_state());
        let (status, _) = get_json(app.clone(), "/v1/models", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = get_json(app.clone(), "/v1/models", Some("wrong")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, body) = get_json(app, "/v1/models", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"][0]["id"], "test-model");
    }

    #[tokio::test]
    async fn test_capabilities_and_sessions_crud() {
        let state = test_state();
        let token = "sekret";
        let app = router(state.clone());

        let (status, body) = get_json(app.clone(), "/v1/capabilities", Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["endpoints"]["chat_completions"], true);

        // Create → list → get → messages → delete.
        let request = axum::http::Request::builder()
            .uri("/api/sessions")
            .method("POST")
            .header("authorization", format!("Bearer {}", token))
            .header("content-type", "application/json")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let created: Value = serde_json::from_slice(&body_bytes).unwrap();
        let session_id = created["id"].as_str().unwrap().to_string();

        let (status, body) = get_json(app.clone(), "/api/sessions?limit=10", Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].as_array().unwrap().iter().any(|s| s["id"] == session_id));

        let (status, body) = get_json(
            app.clone(),
            &format!("/api/sessions/{}", session_id),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "gateway");

        let (status, body) = get_json(
            app.clone(),
            &format!("/api/sessions/{}/messages", session_id),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"].as_array().unwrap().len(), 0);

        let request = axum::http::Request::builder()
            .uri(&format!("/api/sessions/{}", session_id))
            .method("DELETE")
            .header("authorization", format!("Bearer {}", token))
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let (status, _) = get_json(app, &format!("/api/sessions/{}", session_id), Some(token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_runs_404_and_validation() {
        let app = router(test_state());
        let (status, _) = get_json(app.clone(), "/v1/runs/does-not-exist", Some("sekret")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let request = axum::http::Request::builder()
            .uri("/v1/runs")
            .method("POST")
            .header("authorization", "Bearer sekret")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"message": "  "}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_responses_validation_and_404() {
        let app = router(test_state());
        // Missing user message -> 400.
        let request = axum::http::Request::builder()
            .uri("/v1/responses")
            .method("POST")
            .header("authorization", "Bearer sekret")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"input": []}"#))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Unknown previous_response_id -> 404.
        let request = axum::http::Request::builder()
            .uri("/v1/responses")
            .method("POST")
            .header("authorization", "Bearer sekret")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"input": "hi", "previous_response_id": "resp_missing"}"#,
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // GET unknown response -> 404.
        let (status, _) = get_json(app.clone(), "/v1/responses/resp_missing", Some("sekret")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_run_events_404_for_unknown_run() {
        let app = router(test_state());
        let request = axum::http::Request::builder()
            .uri("/v1/runs/missing/events")
            .method("GET")
            .header("authorization", "Bearer sekret")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    struct FakeStreamProvider;

    #[async_trait::async_trait]
    impl crate::provider::Provider for FakeStreamProvider {
        async fn chat_completion(
            &self,
            request: crate::provider::ProviderRequest,
        ) -> crate::error::Result<crate::provider::ProviderResponse> {
            Ok(crate::provider::ProviderResponse {
                content: Some("Hello".into()),
                tool_calls: vec![],
                usage: Some(crate::provider::Usage::default()),
                model: request.model,
                reasoning: None,
                finish_reason: Some("stop".into()),
            })
        }
        async fn chat_completion_stream(
            &self,
            _request: crate::provider::ProviderRequest,
        ) -> crate::error::Result<crate::provider::ProviderStream> {
            let chunks = vec![
                Ok(crate::provider::StreamChunk {
                    delta_content: Some("Hel".into()),
                    ..Default::default()
                }),
                Ok(crate::provider::StreamChunk {
                    delta_content: Some("lo".into()),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }),
            ];
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
        fn supports_streaming(&self) -> bool {
            true
        }
        fn model(&self) -> &str {
            "fake-stream"
        }
        fn name(&self) -> &str {
            "fake"
        }
    }

    fn streaming_state() -> Arc<GatewayState> {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let agent =
            Agent::new(Arc::new(FakeStreamProvider), ToolRegistry::new()).with_store(store);
        GatewayState::new(
            Arc::new(agent),
            "fake-stream".into(),
            "fake".into(),
            None,
            ApprovalRouter::new(),
        )
        .expect("state builds")
    }

    #[tokio::test]
    async fn test_chat_completions_stream_sse() {
        let app = router(streaming_state());
        let request = axum::http::Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"stream": true, "messages": [{"role": "user", "content": "Hi"}]}"#,
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ctype = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(ctype.starts_with("text/event-stream"), "got {}", ctype);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.contains(r#""delta":{"role":"assistant"}"#), "role chunk: {}", text);
        assert!(text.contains(r#""content":"Hel""#), "first delta: {}", text);
        assert!(text.contains(r#""content":"lo""#), "second delta: {}", text);
        assert!(text.contains(r#""finish_reason":"stop""#), "finish: {}", text);
        assert!(text.contains("data: [DONE]"), "done sentinel: {}", text);
    }

    #[tokio::test]
    async fn test_session_chat_stream_sse() {
        let state = streaming_state();
        let sid = state
            .store
            .create_session("stream-test", Some("fake-stream"), None)
            .expect("session created");
        let app = router(state);
        let request = axum::http::Request::builder()
            .uri(format!("/api/sessions/{}/chat/stream", sid))
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"message": "Hi"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.contains(r#""content":"Hel""#), "delta: {}", text);
        assert!(text.contains("data: [DONE]"), "done: {}", text);
    }

    async fn post_chat(app: Router, sid: &str, message: &str) -> Value {
        let request = axum::http::Request::builder()
            .uri(format!("/api/sessions/{}/chat", sid))
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "message": message }).to_string(),
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).expect("json reply")
    }

    #[tokio::test]
    async fn test_session_chat_slash_direct_commands() {
        let state = streaming_state();
        let sid = state
            .store
            .create_session("slash-test", Some("fake-stream"), None)
            .expect("session created");
        let app = router(state.clone());

        let reply = post_chat(app.clone(), &sid, "/help").await;
        assert!(
            reply["response"]
                .as_str()
                .unwrap()
                .starts_with("Gateway slash commands:"),
            "help: {}",
            reply
        );
        assert_eq!(reply["iterations"], 0);

        let reply = post_chat(app.clone(), &sid, "/title Slash Session").await;
        assert_eq!(reply["response"], "title set: Slash Session");
        let reply = post_chat(app.clone(), &sid, "/title").await;
        assert_eq!(reply["response"], "title: Slash Session");

        let reply = post_chat(app.clone(), &sid, "/no-such-command").await;
        assert!(reply["response"]
            .as_str()
            .unwrap()
            .starts_with("unknown command: /no-such-command"));

        // Direct exchanges are persisted to the transcript.
        let messages = state.store.load_messages(&sid).expect("messages load");
        assert!(messages
            .iter()
            .any(|m| m.content.as_deref() == Some("/help")));
        assert!(messages
            .iter()
            .any(|m| m.content.as_deref() == Some("title set: Slash Session")));
    }

    #[tokio::test]
    async fn test_session_chat_slash_skill_expansion() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().to_path_buf();
        let skill_dir = home.join("skills").join("work");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: work\ndescription: Do the work\n---\n\nDo the thing.\n",
        )
        .unwrap();
        let store = Arc::new(
            SqliteSessionStore::open(home.join("state.db")).expect("store opens"),
        );
        let provider = Arc::new(FakeStreamProvider);
        let agent = Agent::new(provider.clone(), ToolRegistry::new())
            .with_store(store)
            .with_context(
                crate::tools::context::ToolContext::new()
                    .with_home(&home)
                    .with_provider(provider),
            );
        let state = GatewayState::new(
            Arc::new(agent),
            "fake-stream".into(),
            "fake".into(),
            None,
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let sid = state
            .store
            .create_session("skill-slash", Some("fake-stream"), None)
            .expect("session created");
        let app = router(state.clone());

        let reply = post_chat(app, &sid, "/work fix the title leak").await;
        assert_eq!(reply["response"], "Hello"); // the fake agent answered

        // The user turn was stored as the hermes skill scaffold, so
        // retitle-skills recognizes it.
        let messages = state.store.load_messages(&sid).expect("messages load");
        let scaffold = messages
            .iter()
            .find(|m| {
                m.content
                    .as_deref()
                    .map(|c| c.starts_with("[IMPORTANT: The user has invoked the \"work\" skill"))
                    .unwrap_or(false)
            })
            .expect("scaffolded user turn stored");
        assert_eq!(
            crate::session::retitle::describe_skill_invocation(scaffold.content.as_ref().unwrap())
                .as_deref(),
            Some("/work — fix the title leak")
        );
    }

    #[tokio::test]
    async fn test_session_chat_stream_slash_direct() {
        let state = streaming_state();
        let sid = state
            .store
            .create_session("slash-stream", Some("fake-stream"), None)
            .expect("session created");
        let app = router(state);
        let request = axum::http::Request::builder()
            .uri(format!("/api/sessions/{}/chat/stream", sid))
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"message": "/help"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.contains("Gateway slash commands:"), "body: {}", text);
        assert!(text.contains("data: [DONE]"), "body: {}", text);
    }

    #[tokio::test]
    async fn test_upload_media_stores_in_cache() {
        // Serialize against tests that temporarily override ULNCLAW_HOME —
        // the upload handler resolves the media cache against the live home.
        let _guard = crate::models_dev::test_env_lock();
        let state = streaming_state();
        let home = state.agent.context().home.clone();
        let app = router(state);
        // Minimal PNG (signature + IHDR + tiny IDAT + IEND; trailing pad
        // bytes are fine — the endpoint hashes, it does not decode).
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let request = axum::http::Request::builder()
            .uri("/api/uploads?name=paste.png")
            .method("POST")
            .header("content-type", "image/png")
            .body(axum::body::Body::from(png.to_vec()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        let path = value["path"].as_str().expect("path returned");
        assert!(path.starts_with(home.to_str().unwrap()), "path: {}", path);
        assert!(std::path::Path::new(path).exists());
        assert_eq!(value["mime"], "image/png");
        assert_eq!(value["bytes"], png.len() as u64);
    }

    struct FakeToolStreamProvider {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for FakeToolStreamProvider {
        async fn chat_completion(
            &self,
            request: crate::provider::ProviderRequest,
        ) -> crate::error::Result<crate::provider::ProviderResponse> {
            Ok(crate::provider::ProviderResponse {
                content: Some("Done.".into()),
                tool_calls: vec![],
                usage: Some(crate::provider::Usage::default()),
                model: request.model,
                reasoning: None,
                finish_reason: Some("stop".into()),
            })
        }
        async fn chat_completion_stream(
            &self,
            _request: crate::provider::ProviderRequest,
        ) -> crate::error::Result<crate::provider::ProviderStream> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                let chunks = vec![
                    Ok(crate::provider::StreamChunk {
                        tool_call_deltas: vec![crate::provider::ToolCallDelta {
                            index: 0,
                            id: Some("call_1".into()),
                            name_delta: Some("echo".into()),
                            arguments_delta: Some("{}".into()),
                        }],
                        ..Default::default()
                    }),
                    Ok(crate::provider::StreamChunk {
                        finish_reason: Some("tool_calls".into()),
                        ..Default::default()
                    }),
                ];
                Ok(Box::pin(futures::stream::iter(chunks)))
            } else {
                let chunks = vec![Ok(crate::provider::StreamChunk {
                    delta_content: Some("Done.".into()),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                })];
                Ok(Box::pin(futures::stream::iter(chunks)))
            }
        }
        fn supports_streaming(&self) -> bool {
            true
        }
        fn model(&self) -> &str {
            "fake-tool-stream"
        }
        fn name(&self) -> &str {
            "fake"
        }
    }

    #[tokio::test]
    async fn test_responses_stream_sse_with_tool_call() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let mut registry = ToolRegistry::new();
        registry
            .register(
                crate::tools::tool("echo")
                    .description("echo tool")
                    .handler(|_args, _ctx| async move { Ok(serde_json::json!({"echoed": true})) })
                    .build()
                    .expect("tool builds"),
            );
        let provider = Arc::new(FakeToolStreamProvider {
            calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        let agent = Agent::new(provider, registry).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "fake-tool-stream".into(),
            "fake".into(),
            None,
            ApprovalRouter::new(),
        )
        .expect("state builds");

        let app = router(state.clone());
        let request = axum::http::Request::builder()
            .uri("/v1/responses")
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"stream": true, "input": "run echo"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&body).to_string();

        assert!(text.contains("event: response.created"), "created: {}", text);
        assert!(text.contains("event: response.output_item.added"), "item added: {}", text);
        assert!(text.contains(r#""type":"function_call""#), "function_call: {}", text);
        assert!(text.contains(r#""type":"function_call_output""#), "fn output: {}", text);
        assert!(text.contains("event: response.output_text.delta"), "text delta: {}", text);
        assert!(text.contains("event: response.output_text.done"), "text done: {}", text);
        assert!(text.contains("event: response.completed"), "completed: {}", text);
        assert!(text.contains(r#""sequence_number""#), "seq: {}", text);

        let responses = state.responses.lock().await;
        assert_eq!(responses.len(), 1);
        let stored = responses.values().next().unwrap();
        assert_eq!(stored["status"], "completed");
    }

    #[tokio::test]
    async fn test_session_chat_stream_tool_cards_sse() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let mut registry = ToolRegistry::new();
        registry
            .register(
                crate::tools::tool("echo")
                    .description("echo tool")
                    .handler(|_args, _ctx| async move { Ok(serde_json::json!({"echoed": true})) })
                    .build()
                    .expect("tool builds"),
            );
        let provider = Arc::new(FakeToolStreamProvider {
            calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        let agent = Agent::new(provider, registry).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "fake-tool-stream".into(),
            "fake".into(),
            None,
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let sid = state
            .store
            .create_session("tool-cards", Some("fake-tool-stream"), None)
            .expect("session created");
        let app = router(state);
        let request = axum::http::Request::builder()
            .uri(format!("/api/sessions/{}/chat/stream", sid))
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"message": "run echo"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.contains("event: hermes.tool.started"), "started: {}", text);
        assert!(text.contains(r#""name":"echo""#), "name: {}", text);
        assert!(text.contains(r#""call_id":"call_1""#), "call id: {}", text);
        assert!(text.contains("event: hermes.tool.completed"), "completed: {}", text);
        assert!(text.contains("echoed"), "result: {}", text);
        assert!(text.contains("data: [DONE]"), "done: {}", text);
    }

    #[tokio::test]
    async fn test_responses_stream_validation() {
        let app = router(streaming_state());
        let request = axum::http::Request::builder()
            .uri("/v1/responses")
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"stream": true, "input": ""}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_approval_timeout_fails_closed() {
        let router = ApprovalRouter::with_options(std::time::Duration::from_millis(50), None);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<PendingApproval>();
        router.register("run-1", "sess-1", tx);
        // Never resolved -> must time out (fail closed), not hang.
        let outcome = router
            .request_outcome("run-1", "test".into(), "sudo whoami".into())
            .await;
        assert_eq!(outcome, ApprovalOutcome::TimedOut);
    }

    #[tokio::test]
    async fn test_approval_always_persisted_and_reloaded() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("approvals.json");
        let router = ApprovalRouter::with_options(
            std::time::Duration::from_secs(5),
            Some(path.clone()),
        );
        router.grant_always("git push --force".into()).await;
        assert!(path.exists());

        // A fresh router reloads the grant and auto-approves without a
        // channel registered.
        let reloaded = ApprovalRouter::with_options(
            std::time::Duration::from_secs(5),
            Some(path),
        );
        let outcome = reloaded
            .request_outcome("no-run", "test".into(), "git push --force".into())
            .await;
        assert_eq!(outcome, ApprovalOutcome::Approved);
    }

    #[test]
    fn test_approvals_config_default() {
        let config = crate::config::ApprovalsConfig::default();
        assert_eq!(config.timeout, 300);
    }

    #[tokio::test]
    async fn test_session_header_extraction() {
        let mut headers = HeaderMap::new();
        headers.insert(SESSION_HEADER, "abc-123".parse().unwrap());
        assert_eq!(session_id_from_headers(&headers), Some("abc-123".into()));
        let mut headers = HeaderMap::new();
        headers.insert("x-hermes-session-id", "legacy".parse().unwrap());
        assert_eq!(session_id_from_headers(&headers), Some("legacy".into()));
        assert_eq!(session_id_from_headers(&HeaderMap::new()), None);
    }

    /// State with a cron store + skills dir attached (phase 9 endpoints).
    fn jobs_state() -> (Arc<GatewayState>, tempfile::TempDir) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Agent::new(provider, ToolRegistry::new()).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "test-model".into(),
            "test".into(),
            Some("sekret".into()),
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let cron = CronStore::open(&temp.path().join("state.db")).expect("cron store opens");
        state.cron.set(Arc::new(cron)).ok();
        state.skills_dir.set(temp.path().join("skills")).ok();
        (state, temp)
    }

    #[tokio::test]
    async fn test_jobs_unavailable_without_store() {
        let app = router(test_state());
        let (status, body) = get_json(app, "/api/jobs", Some("sekret")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body["error"].as_str().unwrap().contains("not enabled"));
    }

    #[tokio::test]
    async fn test_jobs_crud_lifecycle() {
        let (state, _temp) = jobs_state();
        let token = "sekret";
        let app = router(state.clone());

        // Validation errors.
        let (status, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"schedule": "30m", "prompt": "x"}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "Name is required");

        let (status, _) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "j", "schedule": "not-a-schedule", "prompt": "x"}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "j", "schedule": "30m", "prompt": "x", "repeat": 0}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Create.
        let (status, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "daily digest", "schedule": "30m", "prompt": "summarize news", "repeat": 3}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        let job = body["job"].clone();
        let job_id = job["id"].as_str().unwrap().to_string();
        assert_eq!(job["name"], "daily digest");
        assert_eq!(job["enabled"], true);
        assert_eq!(job["repeat"], 3);
        assert!(job["next_run"].as_f64().unwrap() > 0.0);

        // List hides disabled jobs unless asked.
        let (status, body) = get_json(app.clone(), "/api/jobs", Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["jobs"].as_array().unwrap().len(), 1);

        // Get + 404.
        let (status, body) = get_json(app.clone(), &format!("/api/jobs/{}", job_id), Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["job"]["id"], job_id);
        let (status, _) = get_json(app.clone(), "/api/jobs/nope", Some(token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Patch name + schedule; unknown fields are dropped.
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/jobs/{}", job_id), Some(token),
            json!({"name": "renamed", "schedule": "0 9 * * *", "bogus": 1}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["job"]["name"], "renamed");
        assert_eq!(body["job"]["schedule"], "0 9 * * *");

        // Patch with only unknown fields → 400.
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/jobs/{}", job_id), Some(token),
            json!({"bogus": 1}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "No valid fields to update");

        // Pause → hidden from list, visible with include_disabled.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/jobs/{}/pause", job_id), Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["job"]["enabled"], false);
        let (_, body) = get_json(app.clone(), "/api/jobs", Some(token)).await;
        assert_eq!(body["jobs"].as_array().unwrap().len(), 0);
        let (_, body) = get_json(app.clone(), "/api/jobs?include_disabled=true", Some(token)).await;
        assert_eq!(body["jobs"].as_array().unwrap().len(), 1);

        // Resume recomputes next_run.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/jobs/{}/resume", job_id), Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["job"]["enabled"], true);
        assert!(body["job"]["next_run"].as_f64().unwrap() > 0.0);

        // Delete.
        let (status, body) = send_json(
            app.clone(), "DELETE", &format!("/api/jobs/{}", job_id), Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (status, _) = get_json(app.clone(), &format!("/api/jobs/{}", job_id), Some(token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_job_run_creates_tracked_run() {
        let (state, _temp) = jobs_state();
        let token = "sekret";
        let app = router(state.clone());

        let (_, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "runnable", "schedule": "1h", "prompt": "say hi"}),
        ).await;
        let job_id = body["job"]["id"].as_str().unwrap().to_string();

        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/jobs/{}/run", job_id), Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        let run_id = body["run_id"].as_str().expect("run id returned").to_string();

        // The run is tracked; the provider is unreachable so it settles to
        // failed quickly.
        let mut settled = String::new();
        for _ in 0..40 {
            let (_, run) = get_json(app.clone(), &format!("/v1/runs/{}", run_id), Some(token)).await;
            settled = run["status"].as_str().unwrap_or("").to_string();
            if settled == "completed" || settled == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(matches!(settled.as_str(), "completed" | "failed"));

        // The job row recorded the trigger.
        let (_, body) = get_json(app.clone(), &format!("/api/jobs/{}", job_id), Some(token)).await;
        assert!(body["job"]["last_run"].as_f64().is_some());

        // Unknown job → 404.
        let (status, _) = send_json(
            app.clone(), "POST", "/api/jobs/nope/run", Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_jobs_delivery_targets_local_first() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[messaging.telegram]\nenabled = true\n\n[messaging.matrix]\nenabled = true\n",
        )
        .unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let (state, _temp) = jobs_state();
        let app = router(state);
        let (status, body) = get_json(app, "/api/jobs/delivery-targets", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let targets = body["targets"].as_array().unwrap();
        // Implicit local option always first.
        assert_eq!(targets[0]["id"], "local");
        assert_eq!(targets[0]["name"], "Local (save only)");
        assert_eq!(targets[0]["home_target_set"], true);
        let ids: Vec<&str> = targets.iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"telegram"), "enabled telegram listed");
        assert!(ids.contains(&"matrix"), "enabled matrix listed");
        assert!(!ids.contains(&"discord"), "disabled discord hidden");

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_jobs_deliver_persisted_and_updatable() {
        let (state, _temp) = jobs_state();
        let token = "sekret";
        let app = router(state.clone());

        // Explicit platform target accepted and persisted.
        let (status, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "d1", "schedule": "1h", "prompt": "x", "deliver": "telegram"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["job"]["deliver"], "telegram");
        let job_id = body["job"]["id"].as_str().unwrap().to_string();

        // Array deliver normalizes to a comma list.
        let (_, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "d2", "schedule": "1h", "prompt": "x", "deliver": ["telegram", "discord"]}),
        ).await;
        assert_eq!(body["job"]["deliver"], "telegram,discord");

        // Omitted deliver defaults to local.
        let (_, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "d3", "schedule": "1h", "prompt": "x"}),
        ).await;
        assert_eq!(body["job"]["deliver"], "local");

        // PATCH deliver updates it.
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/jobs/{}", job_id), Some(token),
            json!({"deliver": "origin"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["job"]["deliver"], "origin");
        let (_, body) = get_json(app.clone(), &format!("/api/jobs/{}", job_id), Some(token)).await;
        assert_eq!(body["job"]["deliver"], "origin");
    }

    /// Fixed test-only RSA private key (twin of the public half in
    /// `cron::chronos` tests) for signing fire tokens.
    const FIRE_TEST_PRIVATE_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCSvaIDx3Hb8l3N
9XeRqOaS573tRdCt7uuymy03owpSYiTbaWiyWw2muUEPwiywcyLaCChSwRncPyJ9
0Mnd/e1D1NAnST16A1lYl+TO+tDS4ect657i8BxqE7LasjNZUe4uXJ9yFD9nFrgs
mgI6OcZfcQvdjO/ztzEz/ThBWh7LTIyvzp13Dboo/mwCMZXTTslce/ffCDu04mbU
eTV4eJswOUHfUPaZ92KsHYgaZmg2ghwW+i8DdJ7MkKrsO2K86fBhOi5FbVNqUYul
jD2ptKiJDUm+6jkTX+MuLaETQWYFXSVoxke2xD/psL9sDJcWZOYwKL6d/elWkD1P
lCHiJHPRAgMBAAECggEAJVINjqB/GM1/hg5UJruqSNqft2T2OgZ186r7yRayXVmQ
vi0E77ewtSKQpY1hCE+AIavJdaKfDSERiKY9cTRPz9ykRBmghROs+ZdIHkw0KC5E
Oa2fb2BaGbCA4JZJ8QGhbjEobD8yEOn6VX2l62EeTs/VkLdzn6yL2wkf8Z8WDeY7
kp5IIgktXVKZ0xP2lKhcE04EmPj8T3FGzS6AibdsBhjwRSdBFlmE0wqKa2k6lBns
D5m8H1OnE0/tWMJ+YOZHN+29CLwpKcksOWl38h1FUw42EL7oprul7ydS02OLWjwy
sGmVies77zLFYPvseyxMwu8KL5Fn7lzlYQL1zEHDsQKBgQDG35BWdbHaU5oFFwCb
xgkKN4qx0kreBsd6eTwwfI5rQHy778Cc19P4ApuI9HmRxKII9FbKuHzJb3ZRrmN9
MwRhmCtklXxvaw075WBZVERgKbGA3P5hlNcjX1JmvgPNLkCW+qTtJp1zcCgf1h/7
UXgxqUoQ3DR1M6FgXq5a6iRU8wKBgQC85G1LXMERMGDKWzixLigP78TbiW258vMU
MKTRf4Jli8RKASRAe+DFs0klk5eYvlqyWgZt6/4vkPguOfmy/KW5kl2n3CfoRIed
DshI6M19BHEV4G6N3qzsnNQxoQPeDN/IamDou0RhyTwgudgBVdaHMxNPuKOtxCBC
xWV1JjTVKwKBgDVHtA3V3l5Vw4/Vh840EjvwgXH+mxw8yLihPmTnGejWEBTxuLLM
h/eMC0t35BIPkjG/9Hi/UH9PI23iwLjMMEJNWGLMQdg/3/3KCDQmhWMWCH4ztttB
2xmY8iSgh7gyyg8o+4Kls803oShWX58fRopXhoZZ2JwFxxhghWnKDQ3NAoGBAJ5Q
lYnkY6yUb4sqiYl2tf0laEjYFi8TgMgbPQiZZiDV095ytn+VU/5fFZ945EYQxNNW
wKzAbnpPdrLHxJBPUFcIZZaa3pe9WCw6h4MUG6X8YwuC3yXoy+ZES1SNL0CcabL/
9dkZm2aZ0tta57+2webu1/CpQAYTqzZLW42kSAOhAoGBAKiE8vYmA4EcmxngSYEA
cnOnGmFUh6k36PMifq5EQmPJvy1kK5zn2Ay9pOXN0DT89NjTF7n/2+3yQRRWzWDY
ELGypw8hndOvpGrG2zj7MnhECfEcXZcTh3PrBEU/DsYV3EISNN5qh4Uby1zeYqxU
FFkH6vhVrqNHIzR6WyBVZTya
-----END PRIVATE KEY-----"#;

    const FIRE_TEST_PUBLIC_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAkr2iA8dx2/JdzfV3kajm
kue97UXQre7rspstN6MKUmIk22loslsNprlBD8IssHMi2ggoUsEZ3D8ifdDJ3f3t
Q9TQJ0k9egNZWJfkzvrQ0uHnLeue4vAcahOy2rIzWVHuLlyfchQ/Zxa4LJoCOjnG
X3EL3Yzv87cxM/04QVoey0yMr86ddw26KP5sAjGV007JXHv33wg7tOJm1Hk1eHib
MDlB31D2mfdirB2IGmZoNoIcFvovA3SezJCq7DtivOnwYTouRW1TalGLpYw9qbSo
iQ1Jvuo5E1/jLi2hE0FmBV0laMZHtsQ/6bC/bAyXFmTmMCi+nf3pVpA9T5Qh4iRz
0QIDAQAB
-----END PUBLIC KEY-----"#;

    fn sign_fire_token(aud: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let claims = json!({"aud": aud, "exp": now + 300, "purpose": "cron_fire"});
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_rsa_pem(FIRE_TEST_PRIVATE_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_fire_webhook_rejects_without_valid_token() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let (state, _temp) = jobs_state();
        let app = router(state);
        // No chronos config → verification cannot succeed. The route is
        // public (no bearer key needed) but the JWT gate rejects.
        let (status, body) = send_json(
            app.clone(), "POST", "/api/jobs/fire", None,
            json!({"job_id": "anything"}),
        ).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid fire token");
        // Garbage token also 401s.
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/jobs/fire")
            .header("content-type", "application/json")
            .header("authorization", "Bearer not-a-jwt")
            .body(axum::body::Body::from(r#"{"job_id":"x"}"#))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_fire_webhook_lifecycle() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config = format!(
            "[cron.chronos]\nexpected_audience = \"agent:test-instance\"\nnas_jwks_url = '''{}'''\n",
            FIRE_TEST_PUBLIC_PEM
        );
        std::fs::write(dir.path().join("config.toml"), config).unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let (state, _temp) = jobs_state();
        let token = "sekret";
        let app = router(state.clone());

        // Create a job to fire.
        let (_, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "fireable", "schedule": "1h", "prompt": "say hi"}),
        ).await;
        let job_id = body["job"]["id"].as_str().unwrap().to_string();

        let fire_token = sign_fire_token("agent:test-instance");
        let fire_auth = format!("Bearer {}", fire_token);

        // Missing job_id → 400.
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/jobs/fire")
            .header("content-type", "application/json")
            .header("authorization", &fire_auth)
            .body(axum::body::Body::from("{}"))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app.clone(), request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Unknown job → 200 gone (so NAS does not retry).
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/jobs/fire")
            .header("content-type", "application/json")
            .header("authorization", &fire_auth)
            .body(axum::body::Body::from(r#"{"job_id":"nope"}"#))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app.clone(), request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        )
        .unwrap();
        assert_eq!(body["status"], "gone");

        // Real job → 202 accepted, background run dispatched.
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/jobs/fire")
            .header("content-type", "application/json")
            .header("authorization", &fire_auth)
            .body(axum::body::Body::from(serde_json::to_string(&json!({"job_id": job_id})).unwrap()))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app.clone(), request).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        )
        .unwrap();
        assert_eq!(body["status"], "accepted");
        assert_eq!(body["job_id"], job_id);

        // Wrong audience → 401.
        let bad_token = sign_fire_token("agent:someone-else");
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/jobs/fire")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", bad_token))
            .body(axum::body::Body::from(serde_json::to_string(&json!({"job_id": job_id})).unwrap()))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    struct RecordingSender {
        texts: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    #[async_trait::async_trait]
    impl crate::messaging::PlatformSender for RecordingSender {
        async fn send_text(&self, chat_id: &str, text: &str) {
            self.texts
                .lock()
                .unwrap()
                .push((chat_id.to_string(), text.to_string()));
        }
    }

    #[tokio::test]
    async fn test_job_run_delivers_result_via_sender() {
        let texts: Arc<std::sync::Mutex<Vec<(String, String)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        crate::messaging::register_platform_sender(
            "testdeliv",
            Arc::new(RecordingSender { texts: texts.clone() }),
        );

        let (state, _temp) = jobs_state();
        let token = "sekret";
        let app = router(state.clone());

        // Explicit target: platform "testdeliv" is not a known platform,
        // but the explicit `platform:chat` form resolves verbatim.
        let (_, body) = send_json(
            app.clone(), "POST", "/api/jobs", Some(token),
            json!({"name": "delivered", "schedule": "1h", "prompt": "say hi", "deliver": "testdeliv:chat-42"}),
        ).await;
        let job_id = body["job"]["id"].as_str().unwrap().to_string();

        let (status, _) = send_json(
            app.clone(), "POST", &format!("/api/jobs/{}/run", job_id), Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // The provider is unreachable, so the run fails and the failure
        // summary is delivered to the configured target.
        let mut delivered = Vec::new();
        for _ in 0..100 {
            delivered = texts.lock().unwrap().clone();
            if !delivered.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0, "chat-42");
        assert!(delivered[0].1.contains("Cron 'delivered' failed"));
        // Job row: run failed, but delivery itself succeeded.
        let (_, body) = get_json(app.clone(), &format!("/api/jobs/{}", job_id), Some(token)).await;
        assert!(body["job"]["last_status"].as_str().unwrap().starts_with("error:"));
        assert!(body["job"]["last_delivery_error"].is_null());
    }

    #[tokio::test]
    async fn test_skills_listing() {
        let (state, temp) = jobs_state();
        let skill_dir = temp.path().join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo\ndescription: A demo skill\n---\nbody\n",
        )
        .unwrap();

        let app = router(state);
        let (status, body) = get_json(app, "/v1/skills", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["object"], "list");
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["name"], "demo");
        assert_eq!(data[0]["description"], "A demo skill");
    }

    #[tokio::test]
    async fn test_toolsets_listing() {
        let app = router(test_state());
        let (status, body) = get_json(app, "/v1/toolsets", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let data = body["data"].as_array().unwrap();
        assert!(!data.is_empty());
        let coding = data.iter().find(|t| t["name"] == "coding").expect("coding toolset");
        assert!(coding["tools"].as_array().unwrap().len() > 1);
        // test_state's agent has an empty registry → nothing enabled.
        assert_eq!(coding["enabled"], false);
    }

    #[tokio::test]
    async fn test_session_patch_title_and_end_reason() {
        let state = test_state();
        let token = "sekret";
        let app = router(state.clone());

        let id = state.store.create_session("gateway", None, None).unwrap();

        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/sessions/{}", id), Some(token),
            json!({"title": "My session"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["object"], "ulnclaw.session");
        assert_eq!(body["session"]["title"], "My session");

        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/sessions/{}", id), Some(token),
            json!({"end_reason": "done"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["session"]["end_reason"], "done");

        // Unknown fields are rejected.
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/sessions/{}", id), Some(token),
            json!({"title": "x", "hacked": true}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "unsupported_session_field");

        // Newlines collapse to a space (hermes sanitize_title semantics).
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/sessions/{}", id), Some(token),
            json!({"title": "bad\ntitle"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["session"]["title"], "bad title");

        // Overlong title (> 100 chars) → 400.
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/sessions/{}", id), Some(token),
            json!({"title": "x".repeat(101)}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_title");

        let (status, _) = send_json(
            app.clone(), "PATCH", "/api/sessions/missing", Some(token), json!({"title": "x"}),
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_fork() {
        let state = test_state();
        let token = "sekret";
        let app = router(state.clone());

        let id = state.store.create_session("gateway", Some("m"), None).unwrap();
        state
            .store
            .append_message(
                &id,
                &Message {
                    role: Role::User,
                    content: Some("hello fork".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();

        // Fork with explicit id + title.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/sessions/{}/fork", id), Some(token),
            json!({"id": "fork-1", "title": "branch"}),
        ).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["session"]["id"], "fork-1");
        assert_eq!(body["session"]["parent_session_id"], id);
        assert_eq!(body["session"]["title"], "branch");

        // Transcript carried forward.
        let (_, body) = get_json(app.clone(), "/api/sessions/fork-1/messages", Some(token)).await;
        let messages = body["data"].as_array().unwrap();
        assert!(messages.iter().any(|m| m["content"].as_str().unwrap_or("").contains("hello fork")));

        // Source marked branched.
        let source = state.store.get_session_row(&id).unwrap().unwrap();
        assert_eq!(source.end_reason.as_deref(), Some("branched"));

        // Same id again → 409.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/sessions/{}/fork", id), Some(token),
            json!({"id": "fork-1"}),
        ).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "session_exists");

        // Invalid id → 400.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/sessions/{}/fork", id), Some(token),
            json!({"id": "bad\nid"}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_session_id");

        // Missing source → 404.
        let (status, _) = send_json(
            app.clone(), "POST", "/api/sessions/ghost/fork", Some(token), json!({}),
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_capabilities_lists_phase9_endpoints() {
        let app = router(test_state());
        let (status, body) = get_json(app, "/v1/capabilities", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        for flag in ["jobs", "skills", "toolsets", "session_fork", "session_patch"] {
            assert_eq!(body["endpoints"][flag], true, "missing capability {}", flag);
        }
    }

    #[tokio::test]
    async fn test_capabilities_lists_phase10_endpoints() {
        let app = router(test_state());
        let (status, body) = get_json(app, "/v1/capabilities", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        for flag in ["model_options", "session_model_lock"] {
            assert_eq!(body["endpoints"][flag], true, "missing capability {}", flag);
        }
    }

    /// Save/remove/restore canonical provider key env vars so inventory
    /// tests never see ambient credentials.
    struct ProviderEnvScrub {
        saved: Vec<(&'static str, Option<String>)>,
    }
    impl ProviderEnvScrub {
        fn new() -> Self {
            let saved = crate::model_inventory::canonical_key_envs()
                .into_iter()
                .map(|name| (name, std::env::var(name).ok()))
                .collect::<Vec<_>>();
            for (name, _) in &saved {
                std::env::remove_var(name);
            }
            Self { saved }
        }
    }
    impl Drop for ProviderEnvScrub {
        fn drop(&mut self) {
            for (name, value) in self.saved.drain(..) {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    #[tokio::test]
    async fn test_model_options_inventory() {
        // models.dev enrichment is deterministic: pin a file:// registry
        // mirror + cache path under the shared env lock.
        let _guard = crate::models_dev::test_env_lock();
        let _scrub = ProviderEnvScrub::new();
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("models-dev.json");
        std::fs::write(
            &fixture,
            json!({
                "test": {
                    "name": "Test Provider",
                    "api": "https://test.example/v1",
                    "doc": "https://test.example/docs",
                    "models": {
                        "test-model": {
                            "tool_call": true,
                            "reasoning": true,
                            "limit": {"context": 128000, "output": 8192},
                            "cost": {"input": 0.5, "output": 1.5}
                        },
                        "other-model": {"tool_call": true}
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        std::env::set_var(
            crate::models_dev::MODELS_DEV_URL_ENV,
            format!("file://{}", fixture.display()),
        );
        std::env::set_var(
            crate::models_dev::MODELS_DEV_CACHE_ENV,
            dir.path().join("cache.json").display().to_string(),
        );
        crate::models_dev::reset_cache_for_tests();

        let app = router(test_state());
        let (status, body) = get_json(app.clone(), "/api/model/options", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["provider"], "test");
        let providers = body["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["slug"], "test");
        assert_eq!(providers[0]["current"], true);
        // models.dev catalog enrichment for the configured provider row.
        assert_eq!(providers[0]["name"], "Test Provider");
        assert_eq!(providers[0]["catalog"], "models.dev");
        assert_eq!(providers[0]["total_models"], 2);
        let models = providers[0]["models"].as_array().unwrap();
        assert!(models.iter().any(|m| m == "test-model"));
        assert!(models.iter().any(|m| m == "other-model"));
        assert_eq!(providers[0]["capabilities"]["test-model"]["reasoning"], true);
        assert_eq!(providers[0]["capabilities"]["test-model"]["context_window"], 128000);
        assert_eq!(
            providers[0]["capabilities"]["test-model"]["cost"]["input_per_mtok"],
            0.5
        );
        assert!(body["catalog_cache"]["providers"].as_u64().unwrap() >= 1);

        // ?refresh=true forces a registry re-read (same fixture here).
        let (status, body) =
            get_json(app.clone(), "/api/model/options?refresh=true", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["providers"][0]["catalog"], "models.dev");
        assert_eq!(body["providers"][0]["catalog_stale"], false);

        // include_unconfigured appends canonical skeleton rows with
        // picker setup hints.
        let (status, body) = get_json(
            app.clone(),
            "/api/model/options?include_unconfigured=true",
            Some("sekret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let providers = body["providers"].as_array().unwrap();
        assert!(providers.len() > 1);
        let skeleton = providers
            .iter()
            .find(|r| r["slug"] == "anthropic")
            .expect("anthropic skeleton row");
        assert_eq!(skeleton["authenticated"], false);
        assert_eq!(skeleton["auth_type"], "api_key");
        assert_eq!(skeleton["key_env"], "ANTHROPIC_API_KEY");

        // explicit_only keeps only explicitly-configured rows.
        let (status, body) = get_json(
            app.clone(),
            "/api/model/options?include_unconfigured=true&explicit_only=true",
            Some("sekret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let providers = body["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["slug"], "test");

        std::env::remove_var(crate::models_dev::MODELS_DEV_URL_ENV);
        std::env::remove_var(crate::models_dev::MODELS_DEV_CACHE_ENV);
        crate::models_dev::reset_cache_for_tests();
    }

    #[tokio::test]
    async fn test_model_options_config_providers() {
        // `[providers.<slug>]` config entries surface as picker rows.
        let _guard = crate::models_dev::test_env_lock();
        let _scrub = ProviderEnvScrub::new();
        let dir = tempfile::tempdir().unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());
        std::fs::write(
            dir.path().join("config.toml"),
            "[providers.localbox]\nbase_url = \"http://127.0.0.1:9/v1\"\nmodel = \"lm-1\"\n\n[model_catalog]\nexcluded_providers = []\n",
        )
        .unwrap();
        std::env::set_var(
            crate::models_dev::MODELS_DEV_URL_ENV,
            "file:///nonexistent/models-dev.json",
        );
        std::env::set_var(
            crate::models_dev::MODELS_DEV_CACHE_ENV,
            dir.path().join("cache.json").display().to_string(),
        );
        crate::models_dev::reset_cache_for_tests();

        let app = router(test_state());
        let (status, body) = get_json(app.clone(), "/api/model/options", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let providers = body["providers"].as_array().unwrap();
        let local = providers
            .iter()
            .find(|r| r["slug"] == "localbox")
            .expect("localbox row");
        assert_eq!(local["is_user_defined"], true);
        assert_eq!(local["authenticated"], true);
        assert_eq!(local["base_url"], "http://127.0.0.1:9/v1");
        assert_eq!(local["models"], json!(["lm-1"]));

        std::env::remove_var(crate::models_dev::MODELS_DEV_URL_ENV);
        std::env::remove_var(crate::models_dev::MODELS_DEV_CACHE_ENV);
        crate::models_dev::reset_cache_for_tests();
        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_session_model_lock() {
        let state = test_state();
        let token = "sekret";
        let app = router(state.clone());

        let id = state.store.create_session("gateway", Some("test-model"), None).unwrap();

        // Missing model → 400.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/sessions/{}/model", id), Some(token),
            json!({}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "model_required");

        // Lock to another model → acknowledged + persisted.
        let (status, body) = send_json(
            app.clone(), "POST", &format!("/api/sessions/{}/model", id), Some(token),
            json!({"model": "other-model", "provider": "test"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["object"], "ulnclaw.session.model_lock");
        assert_eq!(body["runtime"]["model"], "other-model");
        assert_eq!(body["runtime"]["model_lock"], "accepted");
        let row = state.store.get_session_row(&id).unwrap().unwrap();
        assert_eq!(row.model.as_deref(), Some("other-model"));

        // The override is detected for locked sessions only.
        assert_eq!(session_model_override(&state, &id), Some("other-model".into()));

        // Locking back to the gateway model clears the override.
        let (status, _) = send_json(
            app.clone(), "POST", &format!("/api/sessions/{}/model", id), Some(token),
            json!({"model": "test-model"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(session_model_override(&state, &id), None);

        // Unknown session → 404.
        let (status, _) = send_json(
            app.clone(), "POST", "/api/sessions/ghost/model", Some(token),
            json!({"model": "m"}),
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_model_override_scope_task_local() {
        assert_eq!(crate::agent::current_model_override(), None);
        let inside = crate::agent::model_override_scope("locked".into(), async {
            crate::agent::current_model_override()
        })
        .await;
        assert_eq!(inside.as_deref(), Some("locked"));
        assert_eq!(crate::agent::current_model_override(), None);
    }

    #[tokio::test]
    async fn test_session_recap_endpoint() {
        let state = test_state();
        let token = "sekret";
        let app = router(state.clone());

        let id = state.store.create_session("gateway", None, None).unwrap();
        state
            .store
            .append_message(
                &id,
                &Message {
                    role: Role::User,
                    content: Some("what changed?".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();
        state
            .store
            .append_message(
                &id,
                &Message {
                    role: Role::Assistant,
                    content: Some("I updated the parser.".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();

        let (status, body) = get_json(
            app.clone(),
            &format!("/api/sessions/{}/recap", id),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["object"], "ulnclaw.session.recap");
        assert_eq!(body["session_id"], id);
        let recap = body["recap"].as_str().unwrap();
        assert!(recap.contains("Session recap"));
        assert!(recap.contains("Last ask: what changed?"));
        assert!(recap.contains("Last reply: I updated the parser."));

        let (status, _) = get_json(app.clone(), "/api/sessions/ghost/recap", Some(token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_capabilities_lists_phase15_endpoints() {
        let app = router(test_state());
        let (status, body) = get_json(app, "/v1/capabilities", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["endpoints"]["session_recap"], true);
        assert_eq!(body["endpoints"]["metrics"], true);
    }

    async fn get_text(app: Router, uri: &str, token: Option<&str>) -> (StatusCode, String) {
        let mut request = axum::http::Request::builder().uri(uri).method("GET");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {}", token));
        }
        let response = app
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&body).to_string())
    }

    #[tokio::test]
    async fn test_metrics_endpoint_requires_auth_and_reports() {
        let state = test_state();
        let app = router(state.clone());
        let (status, _) = get_text(app, "/metrics", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let app = router(state.clone());
        let (status, body) = get_text(app, "/metrics", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("ulnclaw_uptime_seconds"));
        assert!(body.contains("ulnclaw_build_info"));
        assert!(body.contains("ulnclaw_sessions_total"));
        assert!(body.contains("ulnclaw_http_requests_total{endpoint=\"chat_completions\"} 0"));

        // A chat completions request bumps the endpoint counter even when
        // the provider itself is unreachable.
        let app = router(state.clone());
        let request = axum::http::Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("authorization", "Bearer sekret")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"messages":[{"role":"user","content":"hi"}]}"#,
            ))
            .unwrap();
        app.oneshot(request).await.unwrap();
        let app = router(state.clone());
        let (status, body) = get_text(app, "/metrics", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("ulnclaw_http_requests_total{endpoint=\"chat_completions\"} 1"));
    }

    #[tokio::test]
    async fn test_usage_endpoint_requires_auth_and_reports() {
        let state = test_state();
        let app = router(state.clone());
        let (status, _) = get_json(app, "/api/usage", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Seed a session with usage so store totals are non-zero.
        let session_id = state.store.create_session("cli", Some("test-model"), None).unwrap();
        state.store.update_usage(&session_id, 120, 45, 3).unwrap();

        let app = router(state.clone());
        let (status, body) = get_json(app, "/api/usage", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["store"]["sessions"], json!(1));
        assert_eq!(body["store"]["input_tokens"], json!(120));
        assert_eq!(body["store"]["output_tokens"], json!(45));
        assert_eq!(body["store"]["total_tokens"], json!(165));
        assert_eq!(body["process"]["total_tokens"], json!(0));
        let sessions = body["sessions"].as_array().expect("sessions array");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["id"], json!(session_id));
        assert_eq!(sessions[0]["total_tokens"], json!(165));

        // limit=0 yields no per-session rows but keeps the totals.
        let app = router(state.clone());
        let (status, body) = get_json(app, "/api/usage?limit=0", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["sessions"].as_array().unwrap().is_empty());
        assert_eq!(body["store"]["sessions"], json!(1));
    }

    #[tokio::test]
    async fn test_config_endpoint_redacts_and_round_trips() {
        // The handler reads/writes config.toml under ULNCLAW_HOME — point
        // it at a temp dir and serialize with other home-overriding tests.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("config.toml"),
            "[model]\nprovider = \"openrouter\"\napi_key = \"sk-super-secret\"\n\n[gateway]\nport = 8642\n",
        )
        .unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // Requires auth.
        let (status, _) = get_json(app.clone(), "/api/config", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // GET redacts secret-looking leaves but keeps structure.
        let (status, body) = get_json(app.clone(), "/api/config", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["config"]["model"]["provider"], "openrouter");
        assert_eq!(body["config"]["model"]["api_key"], "[redacted]");
        assert_eq!(body["config"]["gateway"]["port"], 8642);
        assert!(body["redacted"].as_array().unwrap().iter().any(|v| v == "model.api_key"));

        // PUT: set + unset apply; the redaction placeholder is skipped.
        let (status, body) = send_json(
            app.clone(), "PUT", "/api/config", Some("sekret"),
            json!({
                "set": {
                    "gateway.port": 9999,
                    "model.api_key": "[redacted]",
                    "display.theme": "dark"
                },
                "unset": ["model.provider"],
            }),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["skipped_redacted"], json!(["model.api_key"]));

        // The file on disk reflects the edit; the real secret survived.
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("port = 9999"), "{text}");
        assert!(text.contains("theme = \"dark\""), "{text}");
        assert!(text.contains("api_key = \"sk-super-secret\""), "{text}");
        assert!(!text.contains("openrouter"), "{text}");

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_doctor_endpoint_returns_report() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // Requires auth.
        let (status, _) = get_json(app.clone(), "/api/doctor", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Authenticated: a structured report with sections + issues.
        let (status, body) = get_json(app.clone(), "/api/doctor", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["online"], false);
        let sections = body["report"]["sections"].as_array().expect("sections array");
        assert!(!sections.is_empty(), "doctor report has sections");
        assert!(body["report"]["issues"].is_array());

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_session_export_markdown_and_html() {
        use crate::provider::{Message, Role};
        let state = test_state();
        let app = router(state.clone());
        let id = state
            .store
            .create_session("gateway", Some("test-model"), None)
            .unwrap();
        state
            .store
            .append_message(
                &id,
                &Message {
                    role: Role::User,
                    content: Some("hello <world>".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();

        // Markdown by default, with attachment headers.
        let request = axum::http::Request::builder()
            .uri(&format!("/api/sessions/{id}/export"))
            .header("authorization", "Bearer sekret")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let disposition = response
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(disposition.starts_with("attachment; filename=\"ulnclaw-session-"), "{disposition}");
        assert!(disposition.ends_with(".md\""), "{disposition}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.starts_with("# Session "));
        assert!(body.contains("hello <world>"));

        // HTML variant escapes content.
        let (status, _) = get_json(
            app.clone(),
            &format!("/api/sessions/{id}/export?format=html"),
            Some("sekret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let request = axum::http::Request::builder()
            .uri(&format!("/api/sessions/{id}/export?format=html"))
            .header("authorization", "Bearer sekret")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("hello &lt;world&gt;"));
        assert!(body.contains("<!doctype html>"));

        // Unknown session → 404.
        let (status, _) = get_json(app, "/api/sessions/ghost/export", Some("sekret")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_webhook_subscriptions_crud() {
        // Subscriptions persist under ULNCLAW_HOME — isolate it.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // Empty list to start.
        let (status, body) = get_json(app.clone(), "/api/webhooks/subscriptions", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["subscriptions"], json!([]));

        // Create.
        let (status, body) = send_json(
            app.clone(), "POST", "/api/webhooks/subscriptions", Some("sekret"),
            json!({"name": "Build Events", "description": "CI pings", "events": "push,ci", "deliver": "log"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["name"], "build-events");

        // List shows the row with a webhook URL and masked secret.
        let (status, body) = get_json(app.clone(), "/api/webhooks/subscriptions", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let rows = body["subscriptions"].as_array().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "build-events");
        assert_eq!(rows[0]["events"], json!(["push", "ci"]));
        assert_eq!(rows[0]["url"], format!("{}/webhooks/build-events", body["base_url"].as_str().unwrap()));
        assert_eq!(rows[0]["has_secret"], true);

        // Invalid name → 400.
        let (status, _) = send_json(
            app.clone(), "POST", "/api/webhooks/subscriptions", Some("sekret"),
            json!({"name": "no spaces allowed!"}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Test-fire wiring (unknown name yields the CLI's soft message).
        let (status, body) = send_json(
            app.clone(), "POST", "/api/webhooks/subscriptions/ghost/test", Some("sekret"),
            json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["message"].as_str().unwrap().contains("No subscription"));

        // Delete, then 404 on repeat.
        let (status, body) = send_json(
            app.clone(), "DELETE", "/api/webhooks/subscriptions/build-events", Some("sekret"),
            json!({}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["removed"], "build-events");
        let (status, _) = send_json(
            app.clone(), "DELETE", "/api/webhooks/subscriptions/build-events", Some("sekret"),
            json!({}),
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_monitoring_endpoint_reports_posture() {
        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/monitoring", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app, "/api/monitoring", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        // Default config: health export disabled, OTLP unconfigured.
        assert_eq!(body["enabled"], false);
        assert_eq!(body["otlp"]["enabled"], false);
        assert_eq!(body["otlp"]["endpoint"], Value::Null);
        assert_eq!(body["otlp"]["transport"], "otlp/http-json");
        assert!(body["metrics_interval_seconds"].as_u64().unwrap() >= 5);
        assert!(body["scope"].as_str().unwrap().contains("redacted"));
    }

    #[tokio::test]
    async fn test_logs_tail_endpoint_filters_levels() {
        // gateway.log lives under ULNCLAW_HOME/logs — isolate it.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        std::fs::write(
            dir.path().join("logs/gateway.log"),
            "2026-08-07 10:00:00 INFO ulnclaw_gateway: started\n\
             2026-08-07 10:00:01 WARN ulnclaw_gateway: slow request\n\
             2026-08-07 10:00:02 ERROR ulnclaw_gateway: boom\n",
        )
        .unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // All lines by default.
        let (status, body) = get_json(app.clone(), "/api/logs/tail?lines=10", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["lines"].as_array().unwrap().len(), 3);

        // Level filter keeps WARN+.
        let (status, body) = get_json(app, "/api/logs/tail?lines=10&level=warn", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let lines = body["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].as_str().unwrap().contains("WARN"));
        assert!(lines[1].as_str().unwrap().contains("ERROR"));

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_mcp_servers_list_reports_transport_and_auth() {
        // State with two configured MCP servers: stdio + remote OAuth.
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let mut context = crate::tools::context::ToolContext::default();
        context.config.mcp.servers = vec![
            crate::mcp::McpServerConfig {
                name: "local-tools".into(),
                command: "mcp-server".into(),
                args: vec!["--fast".into()],
                ..Default::default()
            },
            crate::mcp::McpServerConfig {
                name: "remote-api".into(),
                url: Some("https://example.com/mcp".into()),
                auth: Some("oauth".into()),
                ..Default::default()
            },
        ];
        let agent = Agent::new(provider, ToolRegistry::new())
            .with_store(store)
            .with_context(context);
        let state = GatewayState::new(
            Arc::new(agent),
            "test-model".into(),
            "test".into(),
            Some("sekret".into()),
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let app = router(state);

        let (status, _) = get_json(app.clone(), "/api/mcp/servers", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app, "/api/mcp/servers", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let servers = body["servers"].as_array().expect("servers");
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0]["name"], "local-tools");
        assert_eq!(servers[0]["kind"], "stdio");
        assert_eq!(servers[0]["target"], "mcp-server --fast");
        assert_eq!(servers[0]["auth"], "none");
        assert_eq!(servers[1]["kind"], "http");
        assert_eq!(servers[1]["auth"], "oauth");
        assert_eq!(servers[1]["oauth_tokens"], false);
        // No schema-cache entries in the test home → empty tool lists.
        assert_eq!(servers[0]["cached_tools"].as_array().unwrap().len(), 0);
        assert_eq!(servers[1]["cached_tools"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_insights_endpoint_reports_over_store() {
        // The engine reads $ULNCLAW_HOME/state.db directly — seed one.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let store = SqliteSessionStore::open(dir.path().join("state.db")).expect("store");
            let id = store.create_session("cli", Some("test-model"), None).unwrap();
            store
                .append_message(
                    &id,
                    &crate::provider::Message {
                        role: crate::provider::Role::User,
                        content: Some("hello".into()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    },
                )
                .unwrap();
        }
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/insights", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app, "/api/insights?days=7", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["days"], 7);
        assert_eq!(body["empty"], false);
        assert_eq!(body["overview"]["total_sessions"], 1);
        assert!(body["overview"]["total_tokens"].as_i64().unwrap() >= 0);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_storage_endpoints_report_and_optimize() {
        // Seed a temp store so size/count fields have real values; the
        // gateway state must sit on that same store.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(dir.path().join("state.db")).expect("store"),
        );
        let id = store.create_session("cli", Some("test-model"), None).unwrap();
        store
            .append_message(
                &id,
                &crate::provider::Message {
                    role: crate::provider::Role::User,
                    content: Some("storage panel".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Agent::new(provider, ToolRegistry::new()).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "test-model".into(),
            "test".into(),
            Some("sekret".into()),
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let app = router(state);

        let (status, _) = get_json(app.clone(), "/api/storage", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app.clone(), "/api/storage", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["sessions"], 1);
        assert_eq!(body["messages"], 1);
        assert!(body["size_bytes"].as_u64().unwrap() > 0);
        assert!(body["db_path"].as_str().unwrap().ends_with("state.db"));

        let (status, body) =
            post_json(app, "/api/storage/optimize", "{}", "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["merged_indexes"].as_i64().is_some());
        assert!(body["before_bytes"].as_u64().is_some());
        assert!(body["after_bytes"].as_u64().is_some());

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_session_search_finds_seeded_message() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(dir.path().join("state.db")).expect("store"),
        );
        let id = store.create_session("cli", Some("test-model"), None).unwrap();
        store.set_session_title(&id, "searchable session").unwrap();
        store
            .append_message(
                &id,
                &crate::provider::Message {
                    role: crate::provider::Role::User,
                    content: Some("the quick brown fox jumps over the lazy dog".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();

        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Agent::new(provider, ToolRegistry::new()).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "test-model".into(),
            "test".into(),
            Some("sekret".into()),
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let app = router(state);

        let (status, _) = get_json(app.clone(), "/api/sessions/search?q=fox", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = get_json(app.clone(), "/api/sessions/search", Some("sekret")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) =
            get_json(app.clone(), "/api/sessions/search?q=fox", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);
        let first = &body["results"][0];
        assert_eq!(first["title"], "searchable session");
        assert!(first["snippet"].as_str().unwrap().contains("fox"));

        let (status, body) =
            get_json(app, "/api/sessions/search?q=zebra", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 0);
    }

    #[tokio::test]
    async fn test_session_prune_and_archive_endpoints() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SqliteSessionStore::open(dir.path().join("state.db")).expect("store"),
        );
        // Two ended sessions with fresh activity; prune/archive filters
        // only ever see rows with ended_at set.
        let keep = store.create_session("cli", Some("test-model"), None).unwrap();
        let archive_me = store.create_session("cron", Some("test-model"), None).unwrap();
        for id in [&keep, &archive_me] {
            store
                .append_message(
                    id,
                    &crate::provider::Message {
                        role: crate::provider::Role::User,
                        content: Some("hello".into()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    },
                )
                .unwrap();
            store.end_session(id, "ended").unwrap();
        }

        let provider = Arc::new(
            OpenAiProvider::builder()
                .endpoint("http://127.0.0.1:9/v1")
                .model("test-model")
                .name("test")
                .build()
                .expect("provider builds"),
        );
        let agent = Agent::new(provider, ToolRegistry::new()).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "test-model".into(),
            "test".into(),
            Some("sekret".into()),
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let app = router(state);

        // Auth required.
        let (status, _) = post_json(app.clone(), "/api/sessions/prune", "{}", "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Dry-run preview matches both fresh ended sessions.
        let (status, body) = post_json(
            app.clone(),
            "/api/sessions/prune",
            r#"{"newer_than": "1h", "dry_run": true}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["count"], 2);
        assert_eq!(body["candidates"].as_array().unwrap().len(), 2);

        // Archive refuses filter-less requests.
        let (status, _) = post_json(app.clone(), "/api/sessions/archive", "{}", "sekret").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Archive the cron session only.
        let (status, body) = post_json(
            app.clone(),
            "/api/sessions/archive",
            r#"{"newer_than": "1h", "source": "cron"}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["affected"], 1);

        // Archived rows are skipped by prune previews...
        let (status, body) = post_json(
            app.clone(),
            "/api/sessions/prune",
            r#"{"newer_than": "1h", "dry_run": true}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);

        // ...unless include_archived is set.
        let (status, body) = post_json(
            app.clone(),
            "/api/sessions/prune",
            r#"{"newer_than": "1h", "include_archived": true, "dry_run": true}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 2);

        // Real prune deletes the remaining live session.
        let (status, body) = post_json(
            app.clone(),
            "/api/sessions/prune",
            r#"{"newer_than": "1h"}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["affected"], 1);

        // Nothing fresh remains.
        let (status, body) = post_json(
            app,
            "/api/sessions/prune",
            r#"{"newer_than": "1h", "include_archived": true, "dry_run": true}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);
        assert_eq!(body["candidates"][0]["archived"], true);
    }

    #[tokio::test]
    async fn test_backup_snapshots_lifecycle() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        // Seed a critical state file so snapshots have content.
        std::fs::write(dir.path().join("config.toml"), "# test config\n").unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // Auth required.
        let (status, _) = get_json(app.clone(), "/api/backups", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Empty inventory.
        let (status, body) = get_json(app.clone(), "/api/backups", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["snapshots"].as_array().unwrap().len(), 0);

        // Create labeled snapshots (labels keep ids unique within a second).
        for label in ["alpha", "beta", "gamma"] {
            let (status, body) = post_json(
                app.clone(),
                "/api/backups",
                &format!(r#"{{"label": "{label}"}}"#),
                "sekret",
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(body["id"].as_str().unwrap().ends_with(label));
        }
        let (status, body) = get_json(app.clone(), "/api/backups", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let snapshots = body["snapshots"].as_array().unwrap();
        assert_eq!(snapshots.len(), 3);
        assert!(snapshots[0]["files"].as_i64().unwrap() >= 1);

        // Restore a known snapshot.
        let first = snapshots[0]["id"].as_str().unwrap().to_string();
        let (status, body) = post_json(
            app.clone(),
            &format!("/api/backups/{first}/restore"),
            "{}",
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["restored"], true);

        // Unknown snapshot → 404.
        let (status, _) =
            post_json(app.clone(), "/api/backups/nope/restore", "{}", "sekret").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Prune keeps the newest N.
        let (status, body) =
            post_json(app.clone(), "/api/backups/prune", r#"{"keep": 1}"#, "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["removed"], 2);
        let (status, body) = get_json(app, "/api/backups", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["snapshots"].as_array().unwrap().len(), 1);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_curator_endpoints_pin_archive_restore() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        // A minimal installed skill for the usage report to see.
        let skill_dir = dir.path().join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: demo\n---\n\nbody\n").unwrap();
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // Auth + inventory.
        let (status, _) = get_json(app.clone(), "/api/curator", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, body) = get_json(app.clone(), "/api/curator", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let usage = body["usage"].as_array().unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0]["name"], "demo");

        // Pin → archive refused while pinned.
        let (status, _) =
            post_json(app.clone(), "/api/curator/pin", r#"{"skill": "demo"}"#, "sekret").await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) =
            post_json(app.clone(), "/api/curator/archive", r#"{"skill": "demo"}"#, "sekret").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("pinned"));

        // Unpin → archive succeeds → listed as archived.
        let (status, _) =
            post_json(app.clone(), "/api/curator/unpin", r#"{"skill": "demo"}"#, "sekret").await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) =
            post_json(app.clone(), "/api/curator/archive", r#"{"skill": "demo"}"#, "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (status, body) = get_json(app.clone(), "/api/curator", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let archived: Vec<&str> = body["archived"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(archived.contains(&"demo"));

        // Restore brings it back.
        let (status, body) =
            post_json(app.clone(), "/api/curator/restore", r#"{"skill": "demo"}"#, "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (status, body) = get_json(app, "/api/curator", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["archived"].as_array().unwrap().is_empty());
        assert!(skill_dir.join("SKILL.md").exists());

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_checkpoint_endpoints_status_list_restore_prune() {
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        // Auth gate.
        let (status, _) = get_json(app.clone(), "/api/checkpoints/status", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Empty store status.
        let (status, body) =
            get_json(app.clone(), "/api/checkpoints/status", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"]["project_count"], 0);

        // dir is required for listing.
        let (status, _) = get_json(app.clone(), "/api/checkpoints", Some("sekret")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Unknown dir lists empty.
        let (status, body) = get_json(
            app.clone(),
            "/api/checkpoints?dir=/nonexistent-ulnclaw-dir",
            Some("sekret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["checkpoints"].as_array().unwrap().is_empty());

        // Restoring an unknown checkpoint errors cleanly.
        let (status, body) = post_json(
            app.clone(),
            "/api/checkpoints/restore",
            r#"{"dir": "/nonexistent-ulnclaw-dir", "hash": "deadbeef"}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().is_some());

        // Prune over an empty store reports zeros.
        let (status, body) =
            post_json(app.clone(), "/api/checkpoints/prune", r#"{"days": 7}"#, "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["stats"]["scanned"], 0);
        assert_eq!(body["days"], 7);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_plugins_endpoints_inventory_and_toggle() {
        // Isolate ULNCLAW_HOME: disable/enable write config.toml.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/plugins", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app.clone(), "/api/plugins", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["plugins"].as_array().is_some());
        assert!(body["config_hooks"].as_object().is_some());
        assert!(body["disabled"].as_array().is_some());

        // Disable writes the deny-list entry; the inventory reflects it.
        let (status, body) =
            post_json(app.clone(), "/api/plugins/demo/disable", "{}", "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["message"].as_str().unwrap().contains("demo"));
        let (_, body) = get_json(app.clone(), "/api/plugins", Some("sekret")).await;
        let disabled: Vec<String> = body["disabled"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        assert!(disabled.contains(&"demo".to_string()));

        // Enable removes it again.
        let (status, _) =
            post_json(app, "/api/plugins/demo/enable", "{}", "sekret").await;
        assert_eq!(status, StatusCode::OK);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_channels_endpoint_lists_platforms() {
        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/channels", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app, "/api/channels", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let channels = body["channels"].as_array().expect("channels");
        assert!(channels.len() >= 20);
        let names: Vec<&str> = channels
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect();
        assert!(names.contains(&"telegram"));
        assert!(names.contains(&"matrix"));
        assert!(body["enabled_count"].as_u64().is_some());
    }

    #[tokio::test]
    async fn test_egress_status_endpoint_returns_text() {
        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/egress/status", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app, "/api/egress/status", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        let text = body["text"].as_str().expect("text");
        assert!(text.contains("Egress proxy status"));
        assert!(text.contains("Enabled:"));
    }

    #[tokio::test]
    async fn test_system_endpoint_reports_gateway_facts() {
        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/system", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = get_json(app, "/api/system", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["service"], "ulnclaw-gateway");
        assert_eq!(body["version"], crate::VERSION);
        assert!(body["uptime_secs"].as_u64().is_some());
        assert!(body["pid"].as_u64().unwrap() > 0);
        assert!(body["home"].as_str().is_some());
        assert!(body["sessions"].as_u64().is_some());
        assert!(body["plugins_loaded"].as_u64().is_some());
    }

    #[tokio::test]
    async fn test_pairing_endpoints_inventory_and_validation() {
        // Isolate ULNCLAW_HOME: the pairing store lives under <home>/pairing.
        let _guard = crate::models_dev::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let saved_home = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir.path());

        let app = router(test_state());

        let (status, _) = get_json(app.clone(), "/api/pairing", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Fresh home → no pairing activity yet.
        let (status, body) = get_json(app.clone(), "/api/pairing", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["platforms"].as_array().unwrap().len(), 0);

        // Validation: missing fields are rejected.
        let (status, _) =
            post_json(app.clone(), "/api/pairing/approve", "{}", "sekret").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = post_json(
            app.clone(),
            "/api/pairing/approve",
            r#"{"platform": "telegram"}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Approving a code nobody issued → not found.
        let (status, _) = post_json(
            app.clone(),
            "/api/pairing/approve",
            r#"{"platform": "telegram", "code": "ABC123"}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Revoking an unknown pairing → not found.
        let (status, _) = post_json(
            app.clone(),
            "/api/pairing/revoke",
            r#"{"platform": "telegram", "user_id": "u1"}"#,
            "sekret",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Clear-pending over an empty store clears zero.
        let (status, body) =
            post_json(app, "/api/pairing/clear-pending", "{}", "sekret").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["cleared"], 0);

        match saved_home {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    // ------------------------------------------------------------------
    // Browser CDP layer (status / connect / disconnect)
    // ------------------------------------------------------------------

    /// Minimal DevTools discovery server: answers `/json/version` with a
    /// `webSocketDebuggerUrl` so `discover_browser_ws` succeeds.
    async fn spawn_mock_devtools() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let body = format!(
                        "{{\"webSocketDebuggerUrl\":\"ws://127.0.0.1:{port}/devtools/browser/mock\"}}"
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn test_cron_scheduler_runs_due_jobs() {
        // State with the fake provider so the cron run actually completes.
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            SqliteSessionStore::open(&temp.path().join("state.db")).expect("store opens"),
        );
        std::mem::forget(temp);
        let agent =
            Agent::new(Arc::new(FakeStreamProvider), ToolRegistry::new()).with_store(store);
        let state = GatewayState::new(
            Arc::new(agent),
            "fake-stream".into(),
            "test".into(),
            None,
            ApprovalRouter::new(),
        )
        .expect("state builds");
        let dir = tempfile::tempdir().unwrap();
        let cron_store = Arc::new(
            CronStore::open(&dir.path().join("state.db")).expect("cron store opens"),
        );
        state.cron.set(cron_store.clone()).ok();
        let job = CronJob {
            id: "sched-1".into(),
            name: "sched".into(),
            schedule: "60s".into(),
            prompt: "say hello".to_string(),
            skills: vec![],
            enabled: true,
            repeat: None,
            next_run: Some(crate::gateway::now_secs() - 5.0),
            created_at: crate::gateway::now_secs(),
            last_run: None,
            last_status: None,
            deliver: None,
            origin: None,
            last_delivery_error: None,
        };
        cron_store.add(&job).unwrap();

        let handle = spawn_cron_scheduler(state.clone(), 5).expect("scheduler spawns");
        // Scheduler dispatches the due job as a tracked run; the fake
        // provider completes the turn, and the outcome recorder marks the
        // job ok.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let status = cron_store
                .get("sched-1")
                .unwrap()
                .and_then(|j| j.last_status.clone());
            if status.as_deref() == Some("ok") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "job never completed (last: {:?})",
                status
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        handle.abort();
        // The run executed and a cron-run session exists.
        let runs = state.runs.lock().await;
        let run = runs.values().next().expect("run recorded");
        assert_eq!(run.status, "completed");
        assert_eq!(run.result.as_deref(), Some("Hello"));
    }

    async fn post_json(app: Router, uri: &str, body: &str, token: &str) -> (StatusCode, Value) {
        let request = axum::http::Request::builder()
            .uri(uri)
            .method("POST")
            .header("authorization", format!("Bearer {}", token))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn test_browser_status_connect_disconnect() {
        // The override slot is process-global; keep this test the only
        // consumer of it (serialized within this single function).
        crate::browser::clear_cdp_override();
        let app = router(test_state());
        let token = "sekret";

        // Default: unconfigured.
        let (status, body) = get_json(app.clone(), "/v1/browser/status", Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["configured"], false);
        assert_eq!(body["mode"], "none");

        // Connect with an unreachable endpoint -> 502 and no override.
        let (status, body) = post_json(
            app.clone(),
            "/v1/browser/connect",
            "{\"url\":\"http://127.0.0.1:9\"}",
            token,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body["error"]["message"].as_str().unwrap().contains("unreachable"));
        let (_, body) = get_json(app.clone(), "/v1/browser/status", Some(token)).await;
        assert_eq!(body["configured"], false);

        // Connect against the mock discovery endpoint -> verified override.
        let port = spawn_mock_devtools().await;
        let url = format!("http://127.0.0.1:{port}");
        let (status, body) = post_json(
            app.clone(),
            "/v1/browser/connect",
            &format!("{{\"url\":\"{url}\"}}"),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], true);
        let (_, body) = get_json(app.clone(), "/v1/browser/status", Some(token)).await;
        assert_eq!(body["configured"], true);
        assert_eq!(body["source"], "override");
        assert_eq!(body["mode"], "endpoint");

        // Disconnect clears the override.
        let (status, body) = post_json(app.clone(), "/v1/browser/disconnect", "{}", token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["disconnected"], true);
        let (_, body) = get_json(app.clone(), "/v1/browser/status", Some(token)).await;
        assert_eq!(body["configured"], false);
        crate::browser::clear_cdp_override();
    }

    // ------------------------------------------------------------------
    // /p/<profile> multiplexing (hermes api_server parity)
    // ------------------------------------------------------------------

    fn multiplex_app(multiplex: bool, profiles: &[&str]) -> Router {
        let state = test_state();
        let default_router = router(state.clone());
        let builder: ProfileRouterBuilder = Arc::new(|_name: String| {
            Box::pin(async move { Ok(router(test_state())) })
        });
        let hub = ProfileHub::new(
            multiplex,
            profiles.iter().map(|s| s.to_string()).collect(),
            default_router,
            builder,
            None,
        );
        let mirror = Router::new()
            .route(
                "/p/:profile/*rest",
                get(profile_dispatch).post(profile_dispatch),
            )
            .with_state(hub);
        router(state).merge(mirror)
    }

    #[tokio::test]
    async fn test_multiplex_off_prefix_served_by_default() {
        let app = multiplex_app(false, &[]);
        // Prefix accepted but ignored — default profile serves it.
        let (status, body) = get_json(app, "/p/anything/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn test_multiplex_on_unknown_profile_404() {
        let app = multiplex_app(true, &["work"]);
        let (status, body) = get_json(app, "/p/nope/health", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Unknown or unconfigured profile");
    }

    #[tokio::test]
    async fn test_multiplex_on_known_profile_routed() {
        let app = multiplex_app(true, &["work"]);
        let (status, body) = get_json(app.clone(), "/p/work/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        // Second hit reuses the cached profile router.
        let (status, _) = get_json(app, "/p/work/health", None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_multiplex_mirror_enforces_auth() {
        let app = multiplex_app(true, &["work"]);
        // No token → 401 even through the mirror.
        let (status, _) = get_json(app.clone(), "/p/work/v1/models", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // With the bearer token the mirrored route answers.
        let (status, body) = get_json(app, "/p/work/v1/models", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].is_array());
    }

    #[tokio::test]
    async fn test_multiplex_native_routes_unaffected() {
        let app = multiplex_app(true, &["work"]);
        let (status, body) = get_json(app, "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn test_multiplex_mirror_installs_secret_scope() {
        // Pin the process-global multiplex flag for this test (other
        // suites toggle it; native /probe here must take the lenient path).
        let _multiplex_guard = crate::secret_scope::test_multiplex_lock();
        crate::secret_scope::set_multiplex_active(false);
        // Probe route reports the scoped resolution of a probe variable.
        async fn probe() -> String {
            match crate::secret_scope::get_secret("ULNCLAW_SS_MIRROR_PROBE") {
                Ok(Some(value)) => value,
                Ok(None) => "absent".to_string(),
                Err(_) => "unscoped-error".to_string(),
            }
        }
        let probe_router = Router::new().route("/probe", get(probe));
        let builder: ProfileRouterBuilder = Arc::new(|_name: String| {
            Box::pin(async move { Ok(Router::new().route("/probe", get(probe))) })
        });
        let mut scope_map = std::collections::HashMap::new();
        scope_map.insert(
            "ULNCLAW_SS_MIRROR_PROBE".to_string(),
            "scoped".to_string(),
        );
        let hub = ProfileHub::new(
            true,
            ["work"].iter().map(|s| s.to_string()).collect(),
            probe_router.clone(),
            builder,
            Some(Arc::new(move |_name: &str| scope_map.clone())),
        );
        let mirror = Router::new()
            .route(
                "/p/:profile/*rest",
                get(profile_dispatch).post(profile_dispatch),
            )
            .with_state(hub);
        let app = probe_router.merge(mirror);

        // Native route: no scope installed -> process env (unset) -> absent.
        let request = axum::http::Request::builder()
            .uri("/probe")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "absent");

        // Mirror: profile scope installed around the request -> scoped wins.
        let request = axum::http::Request::builder()
            .uri("/p/work/probe")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "scoped");
    }

    #[tokio::test]
    async fn test_cors_echoes_origin_on_regular_request() {
        let app = with_cors(router(test_state()));
        let request = axum::http::Request::builder()
            .uri("/health")
            .method("GET")
            .header("origin", "http://127.0.0.1:5180")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let allow = response
            .headers()
            .get("access-control-allow-origin")
            .expect("cors origin header present")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(allow, "http://127.0.0.1:5180");
    }

    #[tokio::test]
    async fn test_cors_preflight_returns_no_content_and_allow_headers() {
        let app = with_cors(router(test_state()));
        let request = axum::http::Request::builder()
            .uri("/api/sessions")
            .method("OPTIONS")
            .header("origin", "tauri://localhost")
            .header("access-control-request-method", "POST")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let headers = response.headers();
        assert_eq!(
            headers.get("access-control-allow-origin").unwrap().to_str().unwrap(),
            "tauri://localhost"
        );
        assert!(headers
            .get("access-control-allow-methods")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("POST"));
        assert!(headers
            .get("access-control-allow-headers")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Authorization"));
    }

    #[tokio::test]
    async fn test_cors_defaults_to_wildcard_without_origin() {
        let app = with_cors(router(test_state()));
        let request = axum::http::Request::builder()
            .uri("/health")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .unwrap()
                .to_str()
                .unwrap(),
            "*"
        );
    }

    #[test]
    fn dispatcher_lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("dispatcher.lock");
        let first = super::try_acquire_dispatcher_lock_at(&lock_path);
        assert!(first.is_some(), "first acquire must hold the lock");
        // A second open file description in the same process contends.
        assert!(
            super::try_acquire_dispatcher_lock_at(&lock_path).is_none(),
            "second acquire must fail while the first handle lives"
        );
        drop(first);
        assert!(
            super::try_acquire_dispatcher_lock_at(&lock_path).is_some(),
            "lock must be reusable after the handle drops"
        );
    }

    fn notifier_task(status: &str) -> crate::kanban::Task {
        crate::kanban::Task {
            workspace_kind: "scratch".into(),
            workspace_path: None,
            branch_name: None,
            project_id: None,
            session_id: None,
            current_run_id: None,
            block_kind: None,
            block_recurrences: 0,
            goal_mode: false,
            goal_max_turns: None,
            workflow_template_id: None,
            current_step_key: None,
            id: "t_abc".into(),
            board: "default".into(),
            title: "Ship the widget".into(),
            body: String::new(),
            assignee: Some("alice".into()),
            status: status.into(),
            priority: 0,
            created_by: "tester".into(),
            created_at: 0,
            started_at: None,
            completed_at: None,
            tenant: None,
            model: None,
            provider: None,
            result: Some("legacy result".into()),
            claim_lock: None,
            claim_expires: None,
            last_heartbeat_at: None,
            worker_pid: None,
            skills: None,
            max_runtime_seconds: None,
            idempotency_key: None,
            consecutive_failures: 0,
            last_failure_error: None,
            max_retries: None,
        }
    }

    fn notifier_event(kind: &str, payload: serde_json::Value) -> crate::kanban::TaskEvent {
        crate::kanban::TaskEvent {
            id: 1,
            task_id: "t_abc".into(),
            kind: kind.into(),
            payload,
            created_at: 0,
        }
    }

    #[test]
    fn notifier_formats_terminal_messages_like_hermes() {
        let task = notifier_task("done");
        let msg = format_notify_message(
            Some(&task),
            &notifier_event("completed", serde_json::json!({"result": "all green\nsecond line"})),
        )
        .unwrap();
        assert_eq!(
            msg,
            "✔ [default] @alice Kanban t_abc done — Ship the widget\nall green"
        );

        let msg = format_notify_message(
            Some(&task),
            &notifier_event("blocked", serde_json::json!({"reason": "needs api key"})),
        )
        .unwrap();
        assert_eq!(msg, "⏸ [default] @alice Kanban t_abc blocked: needs api key");

        let msg = format_notify_message(
            Some(&task),
            &notifier_event("timed_out", serde_json::json!({"limit_seconds": 600})),
        )
        .unwrap();
        assert_eq!(
            msg,
            "⏱ [default] @alice Kanban t_abc timed out (max_runtime=600s); will retry"
        );

        // Silent kinds advance the cursor without a message.
        assert!(format_notify_message(Some(&task), &notifier_event("archived", serde_json::json!({}))).is_none());
        assert!(format_notify_message(Some(&task), &notifier_event("unblocked", serde_json::json!({}))).is_none());
    }

    #[test]
    fn notifier_completed_prefers_summary_and_falls_back_to_result() {
        let task = notifier_task("done");
        let msg = format_notify_message(
            Some(&task),
            &notifier_event("completed", serde_json::json!({"summary": "handoff first"})),
        )
        .unwrap();
        assert!(msg.ends_with("handoff first"));
        let msg =
            format_notify_message(Some(&task), &notifier_event("completed", serde_json::json!({})))
                .unwrap();
        assert!(msg.ends_with("legacy result"));
    }

    #[tokio::test]
    async fn wake_self_posts_to_chat_completions() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut chunk = vec![0u8; 8192];
            let mut data = Vec::new();
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&data);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let length: usize = text[..header_end]
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if data.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Content-Length: 2\r\n\r\n{}",
                )
                .await
                .unwrap();
            String::from_utf8_lossy(&data).to_string()
        });

        // Wildcard host must connect over loopback (hermes behavior).
        let endpoint = WakeEndpoint {
            host: "0.0.0.0".into(),
            port,
            key: Some("wake-key".into()),
        };
        deliver_session_wake(&endpoint, "sess_123", "[kanban] wake up")
            .await
            .unwrap();
        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(request.contains("POST /v1/chat/completions"));
        // reqwest lowercases header names on the wire.
        assert!(lower.contains("x-ulnclaw-session-id: sess_123"));
        assert!(lower.contains("authorization: bearer wake-key"));
        assert!(request.contains("[kanban] wake up"));
    }

    #[tokio::test]
    async fn wake_fails_fast_on_client_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut chunk = vec![0u8; 8192];
                socket.read(&mut chunk).await.ok();
                let body = b"{\"error\":\"nope\"}";
                let response = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.ok();
                socket.write_all(body).await.ok();
            }
        });
        let endpoint = WakeEndpoint {
            host: "127.0.0.1".into(),
            port,
            key: None,
        };
        let err = deliver_session_wake(&endpoint, "sess_x", "hi")
            .await
            .unwrap_err();
        assert!(err.contains("HTTP 403"), "{err}");
    }

    async fn request_json(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
        token: &str,
    ) -> (StatusCode, Value) {
        let mut builder = axum::http::Request::builder()
            .uri(uri)
            .method(method)
            .header("authorization", format!("Bearer {}", token));
        let payload = match body {
            Some(json) => {
                builder = builder.header("content-type", "application/json");
                axum::body::Body::from(json.to_string())
            }
            None => axum::body::Body::empty(),
        };
        let response = app.oneshot(builder.body(payload).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn test_projects_api_crud_lifecycle() {
        // projects.db resolves through ULNCLAW_HOME — scope it to a temp dir.
        let _guard = crate::models_dev::test_env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", tmp.path());

        let app = router(test_state());
        let token = "sekret";

        // Create (with active flag).
        let (status, body) = request_json(
            app.clone(),
            "POST",
            "/api/projects",
            Some(r#"{"name":"Demo Project","folders":["/tmp/demo-repo"],"use":true}"#),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let slug = body["project"]["slug"].as_str().unwrap().to_string();
        assert_eq!(slug, "demo-project");
        let pid = body["project"]["id"].as_str().unwrap().to_string();

        // List carries the active pointer.
        let (status, body) = get_json(app.clone(), "/api/projects", Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["projects"].as_array().unwrap().len(), 1);
        assert_eq!(body["active_id"].as_str().unwrap(), pid);

        // Get by slug; folder inherited as primary.
        let (status, body) =
            get_json(app.clone(), &format!("/api/projects/{slug}"), Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["project"]["name"], "Demo Project");
        assert_eq!(
            body["project"]["primary_path"].as_str().unwrap(),
            "/tmp/demo-repo"
        );

        // Add a second folder, then promote it.
        let (status, body) = request_json(
            app.clone(),
            "POST",
            &format!("/api/projects/{slug}/folders"),
            Some(r#"{"path":"/tmp/demo-repo-2"}"#),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["project"]["folders"].as_array().unwrap().len(), 2);
        let (status, body) = request_json(
            app.clone(),
            "POST",
            &format!("/api/projects/{slug}/primary"),
            Some(r#"{"path":"/tmp/demo-repo-2"}"#),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["project"]["primary_path"].as_str().unwrap(),
            "/tmp/demo-repo-2"
        );

        // Patch description; unknown project 404s.
        let (status, body) = request_json(
            app.clone(),
            "PATCH",
            &format!("/api/projects/{slug}"),
            Some(r#"{"description":"patched"}"#),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["project"]["description"], "patched");
        let (status, _) =
            get_json(app.clone(), "/api/projects/no-such-project", Some(token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Archive hides it from the default list, restore brings it back.
        let (status, _) = request_json(
            app.clone(),
            "POST",
            &format!("/api/projects/{slug}/archive"),
            None,
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, body) = get_json(app.clone(), "/api/projects", Some(token)).await;
        assert_eq!(body["projects"].as_array().unwrap().len(), 0);
        let (_, body) = get_json(app.clone(), "/api/projects?all=true", Some(token)).await;
        assert_eq!(body["projects"].as_array().unwrap().len(), 1);
        let (status, _) = request_json(
            app.clone(),
            "POST",
            &format!("/api/projects/{slug}/restore"),
            None,
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Clear active, then hard-delete.
        let (status, body) =
            request_json(app.clone(), "POST", "/api/projects/active", Some(r#"{"id":null}"#), token)
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["active_id"].is_null());
        let (status, _) = request_json(
            app.clone(),
            "DELETE",
            &format!("/api/projects/{slug}"),
            None,
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, body) = get_json(app.clone(), "/api/projects", Some(token)).await;
        assert_eq!(body["projects"].as_array().unwrap().len(), 0);

        match prev {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_sessions_list_carries_project_slug() {
        let _guard = crate::models_dev::test_env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", tmp.path());

        // A project whose folder owns the session cwd.
        let workdir = tmp.path().join("work");
        std::fs::create_dir_all(&workdir).unwrap();
        {
            let conn = crate::projects_db::connect(None).unwrap();
            crate::projects_db::create_project(
                &conn,
                &crate::projects_db::CreateProject {
                    name: "Work",
                    slug: Some("work"),
                    folders: &[workdir.to_string_lossy().as_ref()],
                    primary_path: None,
                    description: None,
                    icon: None,
                    color: None,
                    board_slug: None,
                },
            )
            .unwrap();
        }

        let state = test_state();
        state
            .store
            .create_session("cli", None, Some(workdir.to_str().unwrap()))
            .unwrap();
        state.store.create_session("cli", None, Some("/elsewhere")).unwrap();

        let app = router(state);
        let (status, body) = get_json(app, "/api/sessions", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 2);
        let with_project = data.iter().find(|row| row["project"] == "work");
        assert!(with_project.is_some(), "{data:?}");
        let without = data.iter().find(|row| row["cwd"] == "/elsewhere");
        assert!(without.unwrap()["project"].is_null());

        match prev {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn test_projects_repos_and_scan_endpoints() {
        let _guard = crate::models_dev::test_env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", tmp.path());

        let app = router(test_state());
        let token = "sekret";

        // Seed a fake git repo under a scan root.
        let scan_root = tmp.path().join("scan-root");
        std::fs::create_dir_all(scan_root.join("repo-x/.git")).unwrap();

        let (status, body) = request_json(
            app.clone(),
            "POST",
            "/api/projects/scan",
            Some(&format!(
                r#"{{"roots":["{}"],"max_depth":3}}"#,
                scan_root.display()
            )),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["recorded"], 1);

        let (status, body) = get_json(app.clone(), "/api/projects/repos", Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        let repos = body["repos"].as_array().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0]["label"], "repo-x");

        match prev {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    // ------------------------------------------------------------------
    // MCP OAuth dashboard bridge (P242)
    // ------------------------------------------------------------------

    fn mcp_auth_request(name: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(format!("/api/mcp/servers/{}/auth", name))
            .method("POST")
            .header("authorization", "Bearer sekret")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn mcp_auth_validates_server_config() {
        use tower::ServiceExt;

        let _guard = crate::models_dev::test_env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            r#"
[[mcp.servers]]
name = "stdio-srv"
command = "npx"
args = ["-y", "some-server"]

[[mcp.servers]]
name = "header-srv"
url = "https://mcp.example.com/mcp"
[mcp.servers.headers]
Authorization = "Bearer static-token"
"#,
        )
        .unwrap();
        let prev = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", tmp.path());

        let app = router(test_state());

        let response = app.clone().oneshot(mcp_auth_request("nope")).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = app
            .clone()
            .oneshot(mcp_auth_request("stdio-srv"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("env keys"));

        let response = app
            .clone()
            .oneshot(mcp_auth_request("header-srv"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("header/API-key"));

        match prev {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    #[tokio::test]
    async fn mcp_oauth_callback_without_flow_is_open_and_renders_expired() {
        use tower::ServiceExt;

        let app = router(test_state());
        // No bearer token — the route is open (browser redirect).
        let request = axum::http::Request::builder()
            .uri("/api/mcp/oauth/callback/nobody-here-srv?code=c&state=s")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("OAuth flow expired"), "{text}");
    }

    #[tokio::test]
    async fn mcp_oauth_dashboard_flow_end_to_end() {
        use axum::extract::Form;
        use axum::routing::post;
        use tower::ServiceExt;

        let _guard = crate::models_dev::test_env_lock();

        // Fake authorization server.
        async fn as_register() -> axum::Json<Value> {
            axum::Json(json!({"client_id": "dyn-client", "client_secret": null}))
        }
        async fn as_token(
            Form(form): Form<std::collections::HashMap<String, String>>,
        ) -> axum::Json<Value> {
            assert_eq!(form.get("code").map(String::as_str), Some("auth-code-e2e"));
            assert!(form
                .get("redirect_uri")
                .map(|u| u.contains("/api/mcp/oauth/callback/e2e-srv"))
                .unwrap_or(false));
            axum::Json(json!({
                "access_token": "access-e2e",
                "refresh_token": "refresh-e2e",
                "expires_in": 3600
            }))
        }
        let as_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let as_port = as_listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let app = Router::new()
                .route("/register", post(as_register))
                .route("/token", post(as_token));
            axum::serve(as_listener, app).await.ok();
        });

        // Fake Streamable HTTP MCP server gated on the OAuth bearer.
        async fn mcp_rpc(
            headers: HeaderMap,
            axum::extract::Json(req): axum::extract::Json<Value>,
        ) -> (StatusCode, axum::extract::Json<Value>) {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if auth != "Bearer access-e2e" {
                return (StatusCode::UNAUTHORIZED, axum::extract::Json(json!({})));
            }
            let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let Some(id) = req.get("id").cloned() else {
                return (StatusCode::ACCEPTED, axum::extract::Json(Value::Null));
            };
            let result = match method {
                "initialize" => {
                    json!({"serverInfo": {"name": "mock"}, "protocolVersion": "2025-03-26"})
                }
                "tools/list" => json!({"tools": [{
                    "name": "ping",
                    "description": "ping tool",
                    "inputSchema": {"type": "object", "properties": {}}
                }]}),
                _ => json!({}),
            };
            (
                StatusCode::OK,
                axum::extract::Json(json!({"jsonrpc": "2.0", "id": id, "result": result})),
            )
        }
        let mcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mcp_port = mcp_listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(mcp_listener, Router::new().route("/mcp", post(mcp_rpc)))
                .await
                .ok();
        });

        // Config: one OAuth-protected remote server.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            format!(
                r#"
[[mcp.servers]]
name = "e2e-srv"
url = "http://127.0.0.1:{}/mcp"
auth = "oauth"

[[mcp.servers]]
name = "e2e-meta"
url = "http://127.0.0.1:{}/mcp"
"#,
                mcp_port, as_port
            ),
        )
        .unwrap();
        // Point the OAuth metadata at the fake AS (no discovery endpoint
        // on it). The worker reads home from ULNCLAW_HOME.
        let prev = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", tmp.path());
        let as_base = format!("http://127.0.0.1:{}", as_port);
        std::fs::create_dir_all(tmp.path().join("mcp-tokens")).unwrap();
        std::fs::write(
            tmp.path().join("mcp-tokens/e2e-srv.meta.json"),
            format!(
                r#"{{"authorization_endpoint":"{0}/authorize","token_endpoint":"{0}/token","registration_endpoint":"{0}/register"}}"#,
                as_base
            ),
        )
        .unwrap();

        let app = router(test_state());

        // Start the flow.
        let response = app
            .clone()
            .oneshot(mcp_auth_request("e2e-srv"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot["status"], "authorization_required", "{snapshot}");
        let flow_id = snapshot["flow_id"].as_str().unwrap().to_string();
        let auth_url = snapshot["authorization_url"].as_str().expect("auth url");
        let state_value: String = url::Url::parse(auth_url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .expect("state present");

        // Browser redirect hits the open callback route.
        let request = axum::http::Request::builder()
            .uri(format!(
                "/api/mcp/oauth/callback/e2e-srv?code=auth-code-e2e&state={}",
                state_value
            ))
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Authorization received"));

        // Poll the flow status until the worker finishes (token exchange
        // + authenticated tool probe).
        let mut final_snapshot = Value::Null;
        for _ in 0..200 {
            let (status, body) = get_json(
                app.clone(),
                &format!("/api/mcp/oauth/flows/{}", flow_id),
                Some("sekret"),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            if body["status"] == "approved" || body["status"] == "error" {
                final_snapshot = body;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(final_snapshot["status"], "approved", "{final_snapshot}");
        assert_eq!(final_snapshot["tools"][0]["name"], "ping");
        assert!(final_snapshot["error"].is_null());

        // Tokens persisted under the home.
        let tokens: Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("mcp-tokens/e2e-srv.json")).unwrap())
                .unwrap();
        assert_eq!(tokens["access_token"], "access-e2e");

        match prev {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }
}
