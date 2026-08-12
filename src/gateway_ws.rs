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
        "wake.start" => Ok(wake_start()),
        "wake.status" => Ok(wake_status()),
        "wake.stop" => Ok(wake_stop()),
        "pet.generate.status" => Ok(pet_generate_status()),
        "pet.generate" => pet_generate(params).await,
        "pet.hatch" => pet_hatch(params).await,
        "pet.rename" => pet_rename(params),
        "pet.select" => pet_select(params),
        "pet.remove" => pet_remove(params).await,
        "pet.cancel" => pet_cancel(params).await,
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

// ---------------------------------------------------------------------------
// Pet generate / hatch / adopt (hermes tui_gateway pet.* WS parity)
// ---------------------------------------------------------------------------

fn pet_gen_root() -> std::path::PathBuf {
    crate::config::ulnclaw_home().join("pets").join(".gen")
}

fn pet_cancel_flags() -> &'static Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn png_data_uri(img: &image::RgbaImage) -> String {
    use base64::Engine;
    let mut buf = Vec::new();
    if img
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .is_err()
    {
        return String::new();
    }
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(buf)
    )
}

fn pet_generate_status() -> Value {
    match crate::pets_generate::resolve_image_endpoint() {
        Ok(endpoint) => json!({
            "available": true,
            "providers": [{
                "name": endpoint.model,
                "label": format!("OpenAI-compatible images ({})", endpoint.model),
                "default": true,
            }],
        }),
        Err(reason) => json!({ "available": false, "providers": [], "reason": reason }),
    }
}

async fn pet_generate(params: Value) -> Result<Value, String> {
    let prompt = params
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let reference_raw = params
        .get("referenceImage")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if prompt.is_empty() && reference_raw.is_empty() {
        return Err("missing prompt".into());
    }
    let count = params
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or(4)
        .clamp(1, 4) as usize;
    let style = params
        .get("style")
        .and_then(Value::as_str)
        .map(str::to_string);
    let endpoint = crate::pets_generate::resolve_image_endpoint()
        .map_err(|e| crate::pets_generate::humanize_image_error(&e))?;

    let token = uuid::Uuid::new_v4().to_string()[..12].to_string();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    pet_cancel_flags().lock().await.insert(token.clone(), cancel.clone());
    let stage = pet_gen_root().join(&token);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    let reference: Option<std::path::PathBuf> = if reference_raw.is_empty() {
        None
    } else {
        use base64::Engine;
        let comma = reference_raw
            .find(',')
            .ok_or_else(|| "invalid referenceImage data URL".to_string())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&reference_raw[comma + 1..])
            .map_err(|e| format!("decode referenceImage: {e}"))?;
        let path = stage.join("reference.png");
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        Some(path)
    };

    publish("", "pet.generate.progress", json!({ "token": token, "count": count }));

    let concept = if prompt.is_empty() {
        "a pet based on the reference image".to_string()
    } else {
        prompt
    };
    let token_emit = token.clone();
    let cancel_inner = cancel.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let collected: std::sync::Mutex<Vec<(usize, String)>> = std::sync::Mutex::new(Vec::new());
        let stage_for_cb = stage.clone();
        let on_draft = |index: usize, img: &image::RgbaImage| {
            let uri = png_data_uri(img);
            let _ = img.save(stage_for_cb.join(format!("draft-{index}.png")));
            collected.lock().unwrap().push((index, uri.clone()));
            publish(
                "",
                "pet.generate.progress",
                json!({ "token": token_emit, "index": index, "dataUri": uri, "count": count }),
            );
        };
        let reference_arg = reference.as_deref();
        let result = if reference_arg.is_some() {
            // Grounded generation: fan out manually so every draft rides the
            // reference image (generate_base_drafts has no reference input).
            let mut images = Vec::new();
            for index in 0..count {
                if cancel_inner.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let variation = ["front view", "three-quarter view", "side view", "close-up"]
                    [index % 4];
                let prompt = crate::pets_generate::build_base_prompt(&concept, style.as_deref(), variation);
                match crate::pets_generate::generate_image(&endpoint, &prompt, reference_arg, false)
                    .map(|bytes| crate::pets_generate::harden_transparency(&bytes))
                {
                    Ok(img) => {
                        images.push(img.clone());
                        on_draft(index, &img);
                    }
                    Err(error) => tracing::warn!("pet.generate draft {index} failed: {error}"),
                }
            }
            Ok(images)
        } else {
            crate::pets_generate::generate_base_drafts(
                &endpoint,
                &concept,
                count,
                style.as_deref(),
                Some(&on_draft),
                Some(&cancel_inner),
            )
        };
        (result, collected.into_inner().unwrap())
    })
    .await
    .map_err(|e| e.to_string())?;

    pet_cancel_flags().lock().await.remove(&token);
    let (result, mut drafts) = outcome;
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("generation cancelled".into());
    }
    let generated = result.map_err(|e| crate::pets_generate::humanize_image_error(&e))?;
    if drafts.is_empty() && !generated.is_empty() {
        // Callback stream unavailable — fall back to the collected images.
        drafts = generated
            .iter()
            .enumerate()
            .map(|(index, img)| (index, png_data_uri(img)))
            .collect();
    }
    if drafts.is_empty() {
        return Err("generation produced no usable drafts".into());
    }
    drafts.sort_by_key(|(index, _)| *index);
    Ok(json!({
        "ok": true,
        "token": token,
        "drafts": drafts
            .into_iter()
            .map(|(index, data_uri)| json!({ "index": index, "dataUri": data_uri }))
            .collect::<Vec<_>>(),
    }))
}

async fn pet_hatch(params: Value) -> Result<Value, String> {
    let token = params
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let cancel_token = params
        .get("cancelToken")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let cancel_token = if cancel_token.is_empty() { token.clone() } else { cancel_token };
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if token.is_empty() {
        return Err("missing token".into());
    }
    if name.is_empty() {
        return Err("missing name".into());
    }
    let index = params.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let base = pet_gen_root().join(&token).join(format!("draft-{index}.png"));
    if !base.is_file() {
        return Err("draft expired — generate again".into());
    }
    let description = params
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let concept = params
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| name.clone());
    let style = params
        .get("style")
        .and_then(Value::as_str)
        .map(str::to_string);

    let home = crate::config::ulnclaw_home();
    let endpoint = crate::pets_generate::resolve_image_endpoint()
        .map_err(|e| crate::pets_generate::humanize_image_error(&e))?;
    let mut slug = crate::pets::slugify(&name);
    let mut bump = 2;
    while crate::pets::load_pet(&home, &slug).is_some() {
        slug = format!("{}-{}", crate::pets::slugify(&name), bump);
        bump += 1;
    }

    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    pet_cancel_flags()
        .lock()
        .await
        .insert(cancel_token.clone(), cancel.clone());

    let outcome = tokio::task::spawn_blocking(move || {
        let base_image = image::open(&base)
            .map_err(|e| format!("read staged draft: {e}"))?
            .to_rgba8();
        let on_progress = |event: &str, detail: &str| {
            let payload = if event == "row" && detail.matches(':').count() == 2 {
                let mut parts = detail.split(':');
                json!({
                    "event": "row",
                    "state": parts.next().unwrap_or_default(),
                    "done": parts.next().unwrap_or_default(),
                    "total": parts.next().unwrap_or_default(),
                })
            } else {
                json!({ "event": event, "detail": detail })
            };
            publish("", "pet.hatch.progress", payload);
        };
        crate::pets_generate::hatch_pet(
            &home,
            &endpoint,
            &base_image,
            &slug,
            &name,
            &description,
            &concept,
            style.as_deref(),
            Some(&on_progress),
            Some(&cancel),
        )
    })
    .await
    .map_err(|e| e.to_string())?;

    pet_cancel_flags().lock().await.remove(&cancel_token);
    let result = outcome.map_err(|e| crate::pets_generate::humanize_image_error(&e))?;
    Ok(json!({
        "ok": true,
        "slug": result.slug,
        "displayName": result.display_name,
        "warnings": result.validation.warnings,
        "pet": {
            "enabled": false,
            "slug": result.slug,
            "displayName": result.display_name,
            "spritesheet": format!("/api/pets/{}/spritesheet", result.slug),
        },
    }))
}

fn pet_rename(params: Value) -> Result<Value, String> {
    let slug = params
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if slug.is_empty() {
        return Err("missing slug".into());
    }
    if name.is_empty() {
        return Err("missing name".into());
    }
    let home = crate::config::ulnclaw_home();
    let new_slug = crate::pets::rename_pet(&home, &slug, &name)
        .ok_or_else(|| format!("could not rename pet '{slug}'"))?;
    Ok(json!({ "ok": true, "slug": new_slug, "displayName": name }))
}

fn pet_select(params: Value) -> Result<Value, String> {
    let slug = params
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if slug.is_empty() {
        return Err("missing slug".into());
    }
    let home = crate::config::ulnclaw_home();
    let pet = crate::pets::load_pet(&home, &slug)
        .filter(|p| p.exists())
        .ok_or_else(|| format!("'{slug}' is not installed"))?;
    crate::pets::set_active(&slug).map_err(|e| format!("could not persist active pet: {e}"))?;
    Ok(json!({ "ok": true, "slug": slug, "displayName": pet.display_name }))
}

async fn pet_remove(params: Value) -> Result<Value, String> {
    let slug = params
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if slug.is_empty() {
        return Err("missing slug".into());
    }
    let home = crate::config::ulnclaw_home();
    crate::pets::clear_active_if(&slug);
    let removed = crate::pets::remove_pet(&home, &slug);
    Ok(json!({ "ok": removed }))
}

async fn pet_cancel(params: Value) -> Result<Value, String> {
    let token = params
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(flag) = pet_cancel_flags().lock().await.get(&token) {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Wake word (hermes tui_gateway wake.* WS parity)
//
// The ulnclaw build has no always-on mic/STT runtime, so the wake engine
// reports unavailable; the renderer degrades gracefully (slash output shows
// the hint, settings show the wake word as off).
// ---------------------------------------------------------------------------

const WAKE_UNAVAILABLE_HINT: &str =
    "wake-word listening is not built into this ulnclaw gateway (needs a mic + STT runtime)";

fn wake_status() -> Value {
    json!({
        "listening": false,
        "owned_by_caller": false,
        "owner_surface": Value::Null,
        "enabled": false,
        "available": false,
        "audio_silent": false,
        "phrase": "hey ulnclaw",
        "hint": WAKE_UNAVAILABLE_HINT,
        "input_device": { "status": "missing", "error": "no audio input runtime" },
    })
}

fn wake_start() -> Value {
    json!({
        "started": false,
        "reason": "not_available",
        "hint": WAKE_UNAVAILABLE_HINT,
    })
}

fn wake_stop() -> Value {
    json!({ "stopped": false, "reason": "not_owner", "disabled_persisted": false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Agent;
    use crate::provider::openai::OpenAiProvider;
    use crate::session::sqlite::SqliteSessionStore;
    use crate::gateway::ApprovalRouter;
    use crate::tools::ToolRegistry;

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
        GatewayState::new(
            Arc::new(agent),
            "test-model".into(),
            "test".into(),
            Some("sekret".into()),
            ApprovalRouter::new(),
        )
        .expect("state builds")
    }

    #[tokio::test]
    async fn wake_methods_report_unavailable_gracefully() {
        let state = test_state();
        let status = dispatch(state.clone(), "wake.status", json!({})).await.unwrap();
        assert_eq!(status["listening"], false);
        assert_eq!(status["available"], false);
        assert!(status["hint"].as_str().unwrap().contains("not built"));

        let start = dispatch(state.clone(), "wake.start", json!({"persist": true})).await.unwrap();
        assert_eq!(start["started"], false);
        assert!(start["hint"].as_str().is_some());

        let stop = dispatch(state, "wake.stop", json!({})).await.unwrap();
        assert_eq!(stop["stopped"], false);
        assert_eq!(stop["reason"], "not_owner");
    }

    #[tokio::test]
    async fn pet_generate_status_shape() {
        let state = test_state();
        let status = dispatch(state, "pet.generate.status", json!({})).await.unwrap();
        assert!(status.get("available").is_some());
        assert!(status["providers"].is_array());
    }

    #[tokio::test]
    async fn pet_methods_validate_params() {
        let state = test_state();
        assert!(dispatch(state.clone(), "pet.generate", json!({})).await.is_err());
        assert!(dispatch(state.clone(), "pet.hatch", json!({"name": "x"})).await.is_err());
        assert!(dispatch(state.clone(), "pet.hatch", json!({"token": "t"})).await.is_err());
        assert!(
            dispatch(state.clone(), "pet.hatch", json!({"token": "nope", "index": 0, "name": "x"}))
                .await
                .is_err()
        );
        assert!(dispatch(state.clone(), "pet.rename", json!({"slug": "s"})).await.is_err());
        assert!(dispatch(state.clone(), "pet.select", json!({"slug": "ghost"})).await.is_err());

        let removed = dispatch(state.clone(), "pet.remove", json!({"slug": "ghost"})).await.unwrap();
        assert_eq!(removed["ok"], false);

        let cancel = dispatch(state, "pet.cancel", json!({"token": "unknown"})).await.unwrap();
        assert_eq!(cancel["ok"], true);
    }
}
