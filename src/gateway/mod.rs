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
//!   - `GET/POST /api/kanban/boards`, `POST /api/kanban/boards/:slug/switch`,
//!     `GET/POST /api/kanban/tasks`, `GET /api/kanban/tasks/:id`,
//!     `POST /api/kanban/tasks/:id/complete|block|unblock|comment|link|claim`
//!     — kanban board API shared with the CLI + agent tools
//!   - `GET  /api/sessions/:id/recap` — instant local activity recap
//!   - `POST /v1/runs`, `GET /v1/runs`, `GET /v1/runs/{id}`,
//!     `POST /v1/runs/:id/stop`
//!   - `GET/POST /api/jobs`, `GET/PATCH/DELETE /api/jobs/{id}`,
//!     `POST /api/jobs/{id}/pause|resume|run` — cron job management
//!   - `GET /v1/skills`, `GET /v1/toolsets` — discovery endpoints
//!
//! Bearer-token auth via `[gateway] key` / `ULNCLAW_GATEWAY_KEY` (optional;
//! when unset the gateway is open — bind it to localhost).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
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
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(create_response))
        .route(
            "/v1/responses/:id",
            get(get_response).delete(delete_response),
        )
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/sessions/:id",
            get(get_session).patch(patch_session).delete(delete_session),
        )
        .route("/api/sessions/:id/fork", post(fork_session))
        .route("/api/sessions/:id/messages", get(session_messages))
        .route("/api/sessions/:id/chat", post(session_chat))
        .route("/api/sessions/:id/chat/stream", post(session_chat_stream))
        .route("/api/sessions/:id/model", post(lock_session_model))
        .route("/api/sessions/:id/recap", get(session_recap))
        .route("/api/uploads", post(upload_media))
        .route("/api/jobs", get(list_jobs).post(create_job))
        .route(
            "/api/jobs/:id",
            get(get_job).patch(update_job).delete(delete_job),
        )
        .route("/api/pets/config", get(pets::config))
        .route("/api/pets", get(pets::list))
        .route("/api/pets/:slug/spritesheet", get(pets::spritesheet))
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
        .route("/v1/browser/status", get(browser_status))
        .route("/v1/browser/connect", post(browser_connect))
        .route("/v1/browser/disconnect", post(browser_disconnect))
        .route("/v1/runs", get(list_runs).post(start_run))
        .route("/v1/runs/:id", get(get_run))
        .route("/v1/runs/:id/events", get(run_events))
        .route("/v1/runs/:id/approval", post(resolve_approval))
        .route("/v1/runs/:id/stop", post(stop_run));
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
    if config.messaging.webhook.enabled && !config.messaging.webhook.routes.is_empty() {
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
    if config.messaging.bluebubbles.enabled {
        router = router.route("/webhooks/bluebubbles", post(bluebubbles_webhook_route));
        tracing::info!("gateway webhook route mounted: /webhooks/bluebubbles");
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
    use crate::webhook_platforms as wp;
    let ack = |status: &str| -> Response {
        (StatusCode::OK, Json(json!({ "status": status, "route": name }))).into_response()
    };
    let config = &state.agent.context().config;
    let wh = &config.messaging.webhook;
    let Some(route) = wh.routes.iter().find(|r| r.name == name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unknown webhook route '{name}'") })),
        )
            .into_response();
    };
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
    if wp::webhook_rate_limited(&runtime, &route.name, wh.rate_limit, now).await {
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
    if body.is_empty() {
        return StatusCode::OK.into_response();
    }
    let dispatcher = crate::messaging::Dispatcher::new(state.agent.clone(), state.store.clone());
    if let Err(e) =
        crate::webhook_platforms::msgraph_handle_webhook(cfg, &dispatcher, &body, &query).await
    {
        tracing::warn!("msgraph webhook rejected: {e}");
    }
    // Graph requires 2xx to stop notification retries — always ack.
    StatusCode::OK.into_response()
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

/// Multiplex hub: default router + per-profile router cache + policy.
pub struct ProfileHub {
    /// `[gateway] multiplex_profiles`.
    multiplex: bool,
    /// Profile names this gateway serves (`[profiles]` keys).
    profiles: std::collections::HashSet<String>,
    /// Default-profile router (serves `/p/*` requests while multiplexing is
    /// off, mirroring hermes' ignore-the-prefix behavior).
    default_router: Router,
    /// Lazily-built per-profile routers.
    cache: tokio::sync::Mutex<HashMap<String, Router>>,
    builder: ProfileRouterBuilder,
}

impl ProfileHub {
    pub fn new(
        multiplex: bool,
        profiles: std::collections::HashSet<String>,
        default_router: Router,
        builder: ProfileRouterBuilder,
    ) -> Arc<Self> {
        Arc::new(Self {
            multiplex,
            profiles,
            default_router,
            cache: tokio::sync::Mutex::new(HashMap::new()),
            builder,
        })
    }

    /// Resolve the router for a `/p/<profile>` request. `None` = unknown
    /// profile while multiplexing is on (caller 404s).
    async fn resolve(&self, profile: &str) -> Option<Router> {
        if !self.multiplex {
            return Some(self.default_router.clone());
        }
        if !self.profiles.contains(profile) {
            return None;
        }
        {
            let cache = self.cache.lock().await;
            if let Some(router) = cache.get(profile) {
                return Some(router.clone());
            }
        }
        let built = (self.builder)(profile.to_string()).await.ok()?;
        let mut cache = self.cache.lock().await;
        cache.insert(profile.to_string(), built.clone());
        Some(built)
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
    let Some(target) = hub.resolve(&profile).await else {
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
    match tower::ServiceExt::oneshot(target, request).await {
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
    axum::serve(listener, app)
        .await
        .map_err(|e| AgentError::config(format!("gateway serve: {}", e)))
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
}

/// Catalog enrichment for the configured provider row (models.dev).
#[derive(Default)]
struct CatalogEnrichment {
    provider_name: Option<String>,
    api: Option<String>,
    doc: Option<String>,
    models: Vec<String>,
    capabilities: Vec<(String, Value)>,
    cache: Option<crate::models_dev::CacheInfo>,
}

fn query_flag(value: Option<&String>) -> bool {
    value
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .unwrap_or(false)
}

/// Resolve the configured provider against the models.dev registry.
/// Runs on the blocking pool: cache hits are memory-only, but a cold
/// start may touch disk/network (hermes keeps picker work off the loop).
fn models_dev_enrichment(provider: &str, refresh: bool) -> CatalogEnrichment {
    let mut out = CatalogEnrichment::default();
    let registry = crate::models_dev::fetch_models_dev_opts(refresh, true);
    out.cache = Some(crate::models_dev::cache_info());
    let mdev_id = crate::models_dev::provider_to_models_dev(provider)
        .map(str::to_string)
        .unwrap_or_else(|| provider.to_string());
    let Some(pdata) = registry.get(&mdev_id).filter(|v| v.is_object()) else {
        return out;
    };
    out.provider_name = pdata
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    out.api = pdata.get("api").and_then(|v| v.as_str()).map(str::to_string);
    out.doc = pdata.get("doc").and_then(|v| v.as_str()).map(str::to_string);
    // Shared catalog filters (hide lists + agentic noise) live in
    // models_dev; these re-hit the fresh in-memory cache, not the network.
    out.models = crate::models_dev::list_provider_models(provider);
    for model_id in &out.models {
        if let Some(info) = crate::models_dev::get_model_info(provider, model_id) {
            let mut caps = json!({
                "reasoning": info.reasoning,
                "tools": info.tool_call,
                "vision": info.supports_vision(),
                "context_window": info.context_window,
                "max_output_tokens": info.max_output,
            });
            if !info.family.is_empty() {
                caps["family"] = json!(info.family);
            }
            if info.has_cost_data() {
                caps["cost"] = json!({
                    "input_per_mtok": info.cost_input,
                    "output_per_mtok": info.cost_output,
                });
            }
            out.capabilities.push((model_id.clone(), caps));
        }
    }
    out
}

/// `GET /api/model/options` — provider/model inventory for pickers
/// (hermes `_handle_model_options`). The configured provider row is
/// enriched from the models.dev catalog when the provider is known there
/// (model list + per-model capabilities/costs). `?refresh=true` forces a
/// catalog refresh, mirroring hermes' inventory endpoint.
async fn model_options(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ModelOptionsQuery>,
) -> Json<Value> {
    let refresh = query_flag(query.refresh.as_ref());
    let provider = state.provider_name.clone();
    let enrichment = tokio::task::spawn_blocking(move || models_dev_enrichment(&provider, refresh))
        .await
        .unwrap_or_default();

    let mut row = json!({
        "slug": state.provider_name,
        "models": [state.model_name],
        "total_models": 1,
        "is_user_defined": true,
        "authenticated": true,
        "current": true,
    });
    if let Some(name) = &enrichment.provider_name {
        row["name"] = json!(name);
    }
    if !enrichment.models.is_empty() {
        row["models"] = json!(enrichment.models);
        row["total_models"] = json!(enrichment.models.len());
        row["catalog"] = json!("models.dev");
        row["catalog_stale"] = json!(enrichment.cache.as_ref().map_or(true, |c| !c.fresh));
        row["capabilities"] =
            Value::Object(enrichment.capabilities.into_iter().collect());
        if let Some(api) = &enrichment.api {
            row["api"] = json!(api);
        }
        if let Some(doc) = &enrichment.doc {
            row["doc"] = json!(doc);
        }
    }

    let mut payload = json!({
        "providers": [row],
        "model": state.model_name,
        "provider": state.provider_name,
    });
    if let Some(cache) = &enrichment.cache {
        payload["catalog_cache"] = json!({
            "providers": cache.providers,
            "age_secs": cache.age_secs.round() as u64,
            "fresh": cache.fresh,
        });
    }
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

async fn list_sessions(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<SessionsQuery>,
) -> Response {
    let limit = query.limit.unwrap_or(50).min(500);
    match state.store.list_session_rows(limit) {
        Ok(rows) => Json(json!({"object": "list", "data": rows})).into_response(),
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
        Ok(Some(row)) => Json(json!(row)).into_response(),
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

/// `POST /api/uploads` — store a binary upload (desktop composer pastes
/// clipboard images here) in the content-addressed media cache and hand
/// back the path reference. The agent inspects it with
/// vision_analyze/read_file — hermes' text-fallback media semantics for
/// surfaces without native multimodal injection.
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
    #[serde(default)]
    deliver: Option<String>,
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
    if let Some(deliver) = request.deliver.as_deref().filter(|d| *d != "local") {
        return jobs_error(
            StatusCode::BAD_REQUEST,
            &format!("Unsupported deliver target: {}", deliver),
        );
    }
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
    // Whitelist of mutable fields (hermes `_UPDATE_ALLOWED_FIELDS`, minus
    // deliver which this port does not persist).
    let allowed = ["name", "schedule", "prompt", "skills", "repeat", "enabled"];
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

/// Dispatch one cron job as a tracked run (shared by `POST /api/jobs/:id/run`
/// and the scheduler): creates the cron-run session + run row, records the
/// outcome back onto the job when the run finishes, and executes the turn
/// inside the cron approval scope. Returns the run id.
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

    // Record the outcome on the job row once the run finishes.
    let job_store = Arc::clone(&store);
    let job_id = job.id.clone();
    let runs = state.runs.clone();
    let outcome_run_id = run_id.clone();
    tokio::spawn(async move {
        let outcome = loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let snapshot = runs
                .lock()
                .await
                .get(&outcome_run_id)
                .map(|run| (run.status.clone(), run.error.clone()));
            match snapshot {
                None => break ("failed".to_string(), Some("run lost".to_string())),
                Some((status, error))
                    if matches!(status.as_str(), "completed" | "failed") =>
                {
                    break (status, error)
                }
                Some(_) => continue,
            }
        };
        if let Ok(Some(mut job)) = job_store.get(&job_id) {
            job.last_status = Some(match outcome.0.as_str() {
                "completed" => "ok".to_string(),
                other => format!(
                    "error: {}",
                    outcome.1.clone().unwrap_or_else(|| other.to_string())
                ),
            });
            job_store.update(&job).ok();
        }
    });

    spawn_tracked_run(state, run_id.clone(), session_id, job.prompt.clone(), true);
    Some(run_id)
}

/// Start the embedded kanban dispatcher loop (hermes hosts the dispatcher
/// in the gateway, ticking every 60 s by default): reclaim stale claims,
/// promote parent-done todos, spawn detached workers for ready tasks.
pub fn spawn_kanban_dispatcher(
    interval_secs: u64,
    max_spawn: usize,
    use_worktrees: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(5)));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let _ = tokio::task::spawn_blocking(move || {
                let Ok(store) = crate::kanban::KanbanStore::open_default() else {
                    return;
                };
                let home = crate::config::ulnclaw_home();
                match store.dispatch_once(
                    |task| crate::kanban::dispatch_spawn(&home, use_worktrees, task),
                    Some(max_spawn.max(1)),
                    false,
                    2,
                ) {
                    Ok(result) if !result.spawned.is_empty() || !result.reclaimed.is_empty() => {
                        tracing::info!(
                            "kanban dispatch: {} reclaimed, {} promoted, {} spawned",
                            result.reclaimed.len(),
                            result.promoted.len(),
                            result.spawned.len()
                        );
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("kanban dispatch tick failed: {e}"),
                }
            })
            .await;
        }
    })
}

/// Start the cron scheduler loop (hermes scheduler): every `poll_secs`
/// dispatch each due job as a tracked cron run. Called from the gateway
/// command once the cron store is wired; does nothing when absent.
pub fn spawn_cron_scheduler(state: Arc<GatewayState>, poll_secs: u64) -> Option<tokio::task::JoinHandle<()>> {
    let store = state.cron.get().cloned()?;
    Some(tokio::spawn(crate::cron::run_scheduler(
        store,
        poll_secs,
        move |job| {
            let state = state.clone();
            async move {
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
            }
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
    tokio::spawn(RUN_ID.scope(
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
    ));
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
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
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

    #[tokio::test]
    async fn test_model_options_inventory() {
        // models.dev enrichment is deterministic: pin a file:// registry
        // mirror + cache path under the shared env lock.
        let _guard = crate::models_dev::test_env_lock();
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

        std::env::remove_var(crate::models_dev::MODELS_DEV_URL_ENV);
        std::env::remove_var(crate::models_dev::MODELS_DEV_CACHE_ENV);
        crate::models_dev::reset_cache_for_tests();
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
}
