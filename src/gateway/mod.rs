//! HTTP gateway — OpenAI-compatible API server.
//!
//! Minimal Rust port of hermes' `gateway/platforms/api_server.py`:
//!   - `GET  /health`, `GET /health/detailed` — probes (always unauthenticated)
//!   - `GET  /v1/models`, `GET /v1/capabilities`
//!   - `POST /v1/chat/completions` — OpenAI Chat Completions format; opt-in
//!     session continuity via the `X-Ulnclaw-Session-Id` header
//!   - `GET/POST /api/sessions`, `GET/DELETE /api/sessions/{id}`,
//!     `GET /api/sessions/:id/messages`, `POST /api/sessions/:id/chat`
//!   - `POST /v1/runs`, `GET /v1/runs`, `GET /v1/runs/{id}`,
//!     `POST /v1/runs/:id/stop`
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
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::error::{AgentError, Result};
use crate::provider::{Message, Role};
use crate::session::sqlite::SqliteSessionStore;
use crate::session::SessionStore;

/// Header carrying the session id (request + response).
pub const SESSION_HEADER: &str = "x-ulnclaw-session-id";

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
}

impl GatewayState {
    /// Build state from a wired agent (store must be attached).
    pub fn new(
        agent: Arc<Agent>,
        model_name: String,
        provider_name: String,
        key: Option<String>,
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
            get(get_session).delete(delete_session),
        )
        .route("/api/sessions/:id/messages", get(session_messages))
        .route("/api/sessions/:id/chat", post(session_chat))
        .route("/v1/runs", get(list_runs).post(start_run))
        .route("/v1/runs/:id", get(get_run))
        .route("/v1/runs/:id/events", get(run_events))
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
            "sessions": true,
            "streaming": false,
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
    if request.stream {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "streaming is not supported by this gateway", "type": "invalid_request_error"}})),
        )
            .into_response();
    }
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
    match state
        .agent
        .run_with_session(&prompt, history_arg, session_id.as_deref())
        .await
    {
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
// /v1/responses — OpenAI Responses API (stateful via previous_response_id)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ResponsesRequest {
    input: Value,
    #[serde(default)]
    previous_response_id: Option<String>,
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

    match state
        .agent
        .run_with_session(&prompt, history, session_id.as_deref())
        .await
    {
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
    match state
        .agent
        .run_with_session(&request.message, history_arg, Some(&id))
        .await
    {
        Ok(result) => Json(json!({
            "session_id": id,
            "response": result.content,
            "iterations": result.iterations,
        }))
        .into_response(),
        Err(e) => server_error(&e.to_string()),
    }
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
    let run = RunState {
        run_id: run_id.clone(),
        status: "running".to_string(),
        session_id: request.session_id.clone(),
        message: request.message.clone(),
        created_at: now_secs(),
        finished_at: None,
        result: None,
        error: None,
        iterations: None,
        stop_requested: false,
    };
    state.runs.lock().await.insert(run_id.clone(), run.clone());

    let runner = state.clone();
    let spawn_run_id = run_id.clone();
    tokio::spawn(async move {
        let history = request
            .session_id
            .as_ref()
            .map(|sid| {
                runner
                    .store
                    .load_messages(sid)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| m.role != Role::System)
                    .collect::<Vec<_>>()
            })
            .filter(|h| !h.is_empty());
        let outcome = runner
            .agent
            .run_with_session(&request.message, history, request.session_id.as_deref())
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
    });

    (StatusCode::ACCEPTED, Json(json!({"run_id": run_id, "status": "running"}))).into_response()
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
        GatewayState::new(Arc::new(agent), "test-model".into(), "test".into(), Some("sekret".into()))
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
}
