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
        }))
    }
}

/// Build the HTTP router (also used by tests).
pub fn router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/health/detailed", get(health_detailed))
        .route("/v1/health", get(health))
        .route("/v1/models", get(models))
        .route("/api/model/options", get(model_options))
        .route("/v1/capabilities", get(capabilities))
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
        .route("/api/jobs", get(list_jobs).post(create_job))
        .route(
            "/api/jobs/:id",
            get(get_job).patch(update_job).delete(delete_job),
        )
        .route("/api/jobs/:id/pause", post(pause_job))
        .route("/api/jobs/:id/resume", post(resume_job))
        .route("/api/jobs/:id/run", post(run_job_now))
        .route("/v1/skills", get(skills_list))
        .route("/v1/toolsets", get(toolsets_list))
        .route("/v1/runs", get(list_runs).post(start_run))
        .route("/v1/runs/:id", get(get_run))
        .route("/v1/runs/:id/events", get(run_events))
        .route("/v1/runs/:id/approval", post(resolve_approval))
        .route("/v1/runs/:id/stop", post(stop_run))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

/// Serve the gateway until interrupted.
pub async fn serve(state: Arc<GatewayState>, host: &str, port: u16) -> Result<()> {
    let app = router(state.clone());
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
    // Health probes are always open (hermes behavior).
    if path == "/health" || path == "/health/detailed" || path == "/v1/health" {
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

/// `GET /api/model/options` — provider/model inventory for pickers
/// (hermes `_handle_model_options`). ulnclaw runs one configured
/// provider, so the inventory is the configured row (no live catalog
/// probing or pricing enrichment).
async fn model_options(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    Json(json!({
        "providers": [{
            "slug": state.provider_name,
            "models": [state.model_name],
            "total_models": 1,
            "is_user_defined": true,
            "authenticated": true,
            "current": true,
        }],
        "model": state.model_name,
        "provider": state.provider_name,
    }))
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
                Some(crate::agent::StreamEvent::ToolStarted { .. })
                | Some(crate::agent::StreamEvent::ToolCompleted { .. }) => {
                    // Chat-completions clients only get hermes.tool.progress.
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

#[derive(Deserialize)]
struct SessionChatRequest {
    message: String,
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
    let history = state
        .store
        .load_messages(&id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role != Role::System)
        .collect::<Vec<_>>();
    let history_arg = if history.is_empty() { None } else { Some(history) };
    let override_model = session_model_override(&state, &id);
    let outcome = await_with_model_override(
        override_model,
        state.agent.run_with_session(&request.message, history_arg, Some(&id)),
    )
    .await;
    match outcome {
        Ok(result) => Json(json!({
            "session_id": id,
            "response": result.content,
            "iterations": result.iterations,
        }))
        .into_response(),
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
    let history = state
        .store
        .load_messages(&id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role != Role::System)
        .collect::<Vec<_>>();
    let history_arg = if history.is_empty() { None } else { Some(history) };
    stream_agent_response(state, request.message, history_arg, Some(id))
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
    store.update(&job).ok();

    // Record the outcome on the job row once the run finishes.
    let job_store = store.clone();
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

    spawn_tracked_run(state, run_id.clone(), session_id, job.prompt.clone());
    Json(json!({"job": job_value(&job), "run_id": run_id})).into_response()
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

    spawn_tracked_run(state, run_id.clone(), session_id.clone(), request.message.clone());

    (StatusCode::ACCEPTED, Json(json!({"run_id": run_id, "status": "running", "session_id": session_id}))).into_response()
}

/// Execute one agent turn as a tracked background run: registers approval
/// routing, pumps pending approvals into run state, and updates the run row
/// when the turn finishes. Shared by `/v1/runs` and `/api/jobs/:id/run`.
fn spawn_tracked_run(state: Arc<GatewayState>, run_id: String, session_id: String, message: String) {
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

    let runner = state.clone();
    let spawn_run_id = run_id.clone();
    tokio::spawn(RUN_ID.scope(run_id.clone(), async move {
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
                    run.result = Some(result.content);
                    run.session_id = result.session_id.or(run.session_id.take());
                    run.iterations = Some(result.iterations);
                }
                Err(e) => {
                    run.status = "failed".to_string();
                    run.error = Some(e.to_string());
                }
            }
            run.finished_at = Some(now_secs());
        }
        drop(runs);
        runner.router.unregister(&spawn_run_id);
    }));
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

        // Invalid title (newline) → 400.
        let (status, body) = send_json(
            app.clone(), "PATCH", &format!("/api/sessions/{}", id), Some(token),
            json!({"title": "bad\ntitle"}),
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
        let app = router(test_state());
        let (status, body) = get_json(app, "/api/model/options", Some("sekret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["provider"], "test");
        let providers = body["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["slug"], "test");
        assert_eq!(providers[0]["models"][0], "test-model");
        assert_eq!(providers[0]["current"], true);
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
}
