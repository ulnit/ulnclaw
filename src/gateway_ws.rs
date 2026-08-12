//! `/api/ws` — dashboard live socket (hermes desktop parity).
//!
//! The hermes desktop renderer speaks JSON-RPC over this socket: requests
//! are `{id, method, params}` frames answered with `{id, result}` /
//! `{id, error:{message}}`, and server-push events travel as
//! `{method:"event", params:{type, session_id, payload}}`. This bridge
//! implements the renderer's method surface (prompt.submit with live
//! message/tool streaming, session lifecycle, config, approvals/clarify/
//! terminal-read responders, process + pet + wake housekeeping) on top of
//! the ulnclaw gateway, and mirrors the desktop event bus onto the wire.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::gateway::GatewayState;

/// `GET /api/ws` — upgrade to the live event socket.
pub async fn dashboard_ws(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<GatewayState>>,
) -> Response {
    let token = params.get("token").cloned();
    if let Some(key) = state.key.as_ref() {
        if token.as_deref() != Some(key.as_str()) {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response();
        }
    }
    ws.on_upgrade(move |socket| serve_socket(socket, state))
}

type Sink = Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>;

async fn send_frame(sink: &Sink, frame: Value) -> bool {
    sink.lock()
        .await
        .send(Message::Text(frame.to_string().into()))
        .await
        .is_ok()
}

async fn serve_socket(socket: WebSocket, state: Arc<GatewayState>) {
    let (sink, mut stream) = socket.split();
    let sink: Sink = Arc::new(Mutex::new(sink));
    let hello = json!({
        "type": "hello",
        "server": "ulnclaw",
        "version": env!("CARGO_PKG_VERSION"),
    });
    if !send_frame(&sink, hello).await {
        return;
    }
    // hermes parity: the renderer seeds live-sync (and demotes its legacy
    // polls) only after a `gateway.ready` event; advertise change events.
    let ready = json!({
        "method": "event",
        "params": {
            "type": "gateway.ready",
            "payload": {"change_events": true, "server": "ulnclaw"},
        },
    });
    if !send_frame(&sink, ready).await {
        return;
    }
    // Pre-existing sessions never fire a live `sessions.changed`; nudge the
    // renderer to pull the sidebar once on connect so history shows up.
    for change in ["sessions.changed", "cron.changed", "platforms.changed"] {
        let frame = json!({
            "method": "event",
            "params": {"type": change, "payload": {}},
        });
        if !send_frame(&sink, frame).await {
            return;
        }
    }
    let mut bus = crate::desktop_bridge::subscribe();
    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(15));
    keepalive.tick().await; // first tick is immediate; drop it
    loop {
        tokio::select! {
            event = bus.recv() => {
                let Ok(event) = event else { return };
                let frame = json!({
                    "method": "event",
                    "params": {
                        "type": event.event,
                        "session_id": event.session_id,
                        "payload": event.payload,
                    },
                });
                if !send_frame(&sink, frame).await {
                    return;
                }
            }
            _ = keepalive.tick() => {
                if !send_frame(&sink, json!({"type": "ping"})).await {
                    return;
                }
            }
            inbound = stream.next() => {
                match inbound {
                    None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return,
                    Some(Ok(Message::Ping(data))) => {
                        if sink.lock().await.send(Message::Pong(data)).await.is_err() {
                            return;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        let Some(id) = frame.get("id").cloned() else {
                            continue;
                        };
                        let method = frame
                            .get("method")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let params = frame.get("params").cloned().unwrap_or(Value::Null);
                        let sink2 = sink.clone();
                        let state2 = state.clone();
                        tokio::spawn(async move {
                            let reply = match dispatch(state2, &method, params).await {
                                Ok(result) => json!({"id": id, "result": result}),
                                Err(message) => json!({"id": id, "error": {"message": message}}),
                            };
                            send_frame(&sink2, reply).await;
                        });
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// In-flight turn registry (busy detection + interrupt)
// ---------------------------------------------------------------------------

struct TurnEntry {
    abort: tokio::task::AbortHandle,
}

fn turns() -> &'static Mutex<HashMap<String, TurnEntry>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<String, TurnEntry>>> = std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn publish(session_id: &str, event: &str, payload: Value) {
    crate::desktop_bridge::publish(session_id, event, &payload);
}

// ---------------------------------------------------------------------------
// Method dispatch
// ---------------------------------------------------------------------------

async fn dispatch(state: Arc<GatewayState>, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "prompt.submit" => prompt_submit(state, params).await,
        "session.interrupt" => session_interrupt(params),
        "session.resume" => session_resume(state, params),
        "session.close" => session_close(state, params),
        "session.title" => session_title(state, params),
        "config.get" => config_get(),
        "config.set" => config_set(params),
        "reload.env" | "reload.mcp" => Ok(json!({"ok": true})),
        "approval.respond" => approval_respond(params),
        "clarify.respond" => clarify_respond(params),
        "sudo.respond" | "secret.respond" => Ok(json!({"status": "ignored"})),
        "terminal.read.respond" => terminal_read_respond(params),
        "process.list" => Ok(process_list()),
        "process.kill" => process_kill(params),
        "model.options" => model_options(state).await,
        "commands.catalog" => Ok(commands_catalog()),
        "complete.slash" => Ok(complete_slash(params)),
        "complete.path" => Ok(complete_path(params)),
        "slash.exec" => Ok(json!({"ok": false, "error": "slash exec runs client-side in ulnclaw"})),
        "llm.oneshot" => llm_oneshot(state, params).await,
        "message.react" => Ok(json!({"ok": true})),
        "wake.pause" | "wake.resume" => Ok(json!({"ok": true})),
        "pet.remove" | "pet.cancel" => Ok(json!({"ok": true})),
        other => Err(format!("unknown method: {other}")),
    }
}

async fn prompt_submit(state: Arc<GatewayState>, params: Value) -> Result<Value, String> {
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if session_id.is_empty() {
        return Err("session_id is required".into());
    }
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    {
        let map = turns().lock().await;
        if map.contains_key(&session_id) {
            return Err("session busy (4009)".into());
        }
    }
    state
        .store
        .ensure_session(&session_id, "desktop", None, None)
        .map_err(|e| e.to_string())?;

    publish(&session_id, "message.start", json!({}));

    let history = state
        .store
        .load_messages(&session_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role != crate::provider::Role::System)
        .collect::<Vec<_>>();
    let history_arg = if history.is_empty() { None } else { Some(history) };

    let runner = state.agent.clone();
    let sid_emit = session_id.clone();
    let sid_run = session_id.clone();
    let task = tokio::spawn(crate::agent::stream_scope(
        Arc::new(move |event: crate::agent::StreamEvent| {
            match event {
                crate::agent::StreamEvent::Delta(delta) => {
                    publish(&sid_emit, "message.delta", json!({"text": delta}));
                }
                crate::agent::StreamEvent::ToolProgress { tool, status } => {
                    publish(&sid_emit, "tool.progress", json!({"name": tool, "status": status}));
                }
                crate::agent::StreamEvent::ToolStarted {
                    name,
                    call_id,
                    arguments,
                } => {
                    publish(
                        &sid_emit,
                        "tool.start",
                        json!({"name": name, "call_id": call_id, "id": call_id, "arguments": arguments, "title": name}),
                    );
                }
                crate::agent::StreamEvent::ToolCompleted { call_id, result } => {
                    publish(
                        &sid_emit,
                        "tool.complete",
                        json!({"call_id": call_id, "id": call_id, "result": result}),
                    );
                }
            }
        }),
        async move {
            runner
                .run_with_session_images(&text, vec![], history_arg, Some(&sid_run))
                .await
        },
    ));
    let abort = task.abort_handle();
    turns()
        .lock()
        .await
        .insert(session_id.clone(), TurnEntry { abort });

    let sid2 = session_id.clone();
    tokio::spawn(async move {
        let outcome = task.await;
        turns().lock().await.remove(&sid2);
        match outcome {
            Ok(Ok(result)) => {
                publish(
                    &sid2,
                    "message.complete",
                    json!({
                        "text": result.content,
                        "usage": {
                            "input": result.usage.prompt_tokens,
                            "output": result.usage.completion_tokens,
                            "total": result.usage.total_tokens,
                            "calls": result.iterations,
                        },
                    }),
                );
            }
            Ok(Err(err)) => {
                publish(
                    &sid2,
                    "message.complete",
                    json!({"text": "", "status": "error", "error": err.to_string()}),
                );
            }
            Err(_) => {
                publish(
                    &sid2,
                    "message.complete",
                    json!({"text": "", "status": "error", "error": "interrupted", "partial": true}),
                );
            }
        }
        publish(&sid2, "session.info", json!({"session_id": sid2, "running": false}));
        publish(&sid2, "sessions.changed", json!({"session_id": sid2}));
    });

    Ok(json!({"ok": true, "session_id": session_id}))
}

fn session_interrupt(params: Value) -> Result<Value, String> {
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let removed = {
        let mut map = match turns().try_lock() {
            Ok(map) => map,
            Err(_) => return Err("turn registry locked".into()),
        };
        map.remove(session_id)
    };
    let interrupted = removed.is_some();
    if let Some(entry) = removed {
        entry.abort.abort();
    }
    Ok(json!({"ok": true, "interrupted": interrupted}))
}

fn session_resume(state: Arc<GatewayState>, params: Value) -> Result<Value, String> {
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if session_id.is_empty() {
        return Err("session_id is required".into());
    }
    let source = params
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("desktop");
    state
        .store
        .ensure_session(&session_id, source, None, None)
        .map_err(|e| e.to_string())?;
    Ok(json!({"session_id": session_id}))
}

fn session_close(state: Arc<GatewayState>, params: Value) -> Result<Value, String> {
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let _ = state.store.end_session(session_id, "closed");
    publish(session_id, "sessions.changed", json!({"session_id": session_id}));
    Ok(json!({"ok": true}))
}

fn session_title(state: Arc<GatewayState>, params: Value) -> Result<Value, String> {
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = params.get("title").and_then(Value::as_str).unwrap_or_default();
    state
        .store
        .set_session_title(session_id, title)
        .map_err(|e| e.to_string())?;
    publish(
        session_id,
        "session.title",
        json!({"session_id": session_id, "title": title}),
    );
    Ok(json!({"ok": true}))
}

fn config_get() -> Result<Value, String> {
    let path = crate::config_cmd::config_path();
    let config = crate::config::UlncLawConfig::load(Some(&path)).unwrap_or_default();
    serde_json::to_value(&config).map_err(|e| e.to_string())
}

fn config_set(params: Value) -> Result<Value, String> {
    // The desktop writes a handful of display/behavior keys; accept and
    // persist the raw patch into config.toml under [desktop].
    let path = crate::config_cmd::config_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml::Value = raw.parse().unwrap_or_else(|_| toml::Value::Table(Default::default()));
    let table = doc
        .as_table_mut()
        .ok_or_else(|| "config root is not a table".to_string())?;
    let desktop = table
        .entry("desktop")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    if let (Some(desktop_table), Some(obj)) = (desktop.as_table_mut(), params.as_object()) {
        for (key, value) in obj {
            if let Ok(converted) = json_to_toml(value) {
                desktop_table.insert(key.clone(), converted);
            }
        }
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())?;
    Ok(json!({"ok": true}))
}

fn json_to_toml(value: &Value) -> Result<toml::Value, ()> {
    Ok(match value {
        Value::Null => toml::Value::String(String::new()),
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else {
                toml::Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => toml::Value::Array(
            items
                .iter()
                .map(json_to_toml)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (key, value) in map {
                table.insert(key.clone(), json_to_toml(value)?);
            }
            toml::Value::Table(table)
        }
    })
}

fn approval_respond(params: Value) -> Result<Value, String> {
    let session_key = params
        .get("request_id")
        .or_else(|| params.get("session_key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let choice = params
        .get("choice")
        .or_else(|| params.get("response"))
        .and_then(Value::as_str)
        .unwrap_or("deny");
    let resolved = crate::approval_gateway::resolve(session_key, choice);
    Ok(json!({"ok": resolved}))
}

fn clarify_respond(params: Value) -> Result<Value, String> {
    let clarify_id = params
        .get("request_id")
        .or_else(|| params.get("clarify_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let response = params
        .get("response")
        .or_else(|| params.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resolved = crate::clarify_gateway::resolve(clarify_id, response);
    Ok(json!({"ok": resolved}))
}

fn terminal_read_respond(params: Value) -> Result<Value, String> {
    let id = params
        .get("request_id")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let answer = params
        .get("text")
        .or_else(|| params.get("answer"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let ok = crate::desktop_bridge::resolve_read(id, Ok(answer));
    Ok(json!({"ok": ok}))
}

fn process_list() -> Value {
    let rows: Vec<Value> = crate::tools::builtin::terminal::list_background_processes()
        .into_iter()
        .map(|info| {
            json!({
                "pid": info.pid,
                "session_id": info.session_id,
                "command": info.command,
                "status": info.status,
                "uptime_seconds": info.uptime_seconds,
                "output_preview": info.output_preview,
            })
        })
        .collect();
    json!(rows)
}

fn process_kill(params: Value) -> Result<Value, String> {
    let process_id = params
        .get("process_id")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let value = crate::tools::builtin::terminal::request_close_terminal("", process_id);
    Ok(value)
}

async fn model_options(state: Arc<GatewayState>) -> Result<Value, String> {
    let response = crate::gateway::model_options_pub(State(state)).await;
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|e| e.to_string())?;
    if !parts.status.is_success() {
        return Err("model options unavailable".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn commands_catalog() -> Value {
    let commands: Vec<Value> = crate::gateway::slash_catalog_pub()
        .into_iter()
        .map(|(name, description)| json!({"name": name, "description": description}))
        .collect();
    json!({"commands": commands})
}

fn complete_slash(params: Value) -> Value {
    let prefix = params
        .get("prefix")
        .or_else(|| params.get("query"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let matches: Vec<Value> = crate::gateway::slash_catalog_pub()
        .into_iter()
        .filter(|(name, _)| name.to_lowercase().contains(&prefix))
        .take(20)
        .map(|(name, description)| json!({"name": name, "description": description}))
        .collect();
    json!({"suggestions": matches})
}

fn complete_path(params: Value) -> Value {
    let prefix = params
        .get("prefix")
        .or_else(|| params.get("query"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (dir_part, file_part) = match prefix.rfind('/') {
        Some(idx) => (&prefix[..idx + 1], &prefix[idx + 1..]),
        None => ("", prefix),
    };
    let base = if dir_part.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        std::path::PathBuf::from(dir_part)
    };
    let mut suggestions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !file_part.is_empty() && !name.to_lowercase().starts_with(&file_part.to_lowercase()) {
                continue;
            }
            let is_dir = entry.path().is_dir();
            let label = if is_dir {
                format!("{dir_part}{name}/")
            } else {
                format!("{dir_part}{name}")
            };
            suggestions.push(json!({"path": label, "is_dir": is_dir}));
            if suggestions.len() >= 20 {
                break;
            }
        }
    }
    json!({"suggestions": suggestions})
}

async fn llm_oneshot(state: Arc<GatewayState>, params: Value) -> Result<Value, String> {
    let prompt = params
        .get("prompt")
        .or_else(|| params.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if prompt.is_empty() {
        return Err("prompt is required".into());
    }
    let runner = state.agent.clone();
    let result = runner
        .run_with_session_images(&prompt, vec![], None, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"text": result.content}))
}
