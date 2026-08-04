//! Platform tools — Home Assistant, kanban, browser, computer-use, and
//! messaging platform stubs.
//!
//! Home Assistant tools are fully implemented (REST API, gated on
//! HASS_URL + HASS_TOKEN). Kanban implements a local coordination board in
//! SQLite. Browser/computer-use/messaging tools register with hermes-faithful
//! schemas but gate on their backends being configured.

use crate::error::Result;
use crate::tools::{tool, ToolAvailability, ToolContext, ToolRegistry};
use serde_json::json;

pub fn register(registry: &mut ToolRegistry) {
    register_homeassistant(registry);
    register_kanban(registry);
    register_browser(registry);
    register_gated_stubs(registry);
}

// ---------------------------------------------------------------------------
// Home Assistant (ha_list_entities, ha_get_state, ha_list_services, ha_call_service)
// ---------------------------------------------------------------------------

fn hass_configured() -> ToolAvailability {
    if crate::config::get_env_value("HASS_TOKEN").is_some()
        && crate::config::get_env_value("HASS_URL").is_some()
    {
        ToolAvailability::available()
    } else {
        ToolAvailability::unavailable("HASS_URL and HASS_TOKEN must be set")
    }
}

async fn hass_request(
    ctx: &std::sync::Arc<ToolContext>,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let url = crate::config::get_env_value("HASS_URL")
        .ok_or_else(|| crate::error::AgentError::tool("HASS_URL not set"))?;
    let token = crate::config::get_env_value("HASS_TOKEN")
        .ok_or_else(|| crate::error::AgentError::tool("HASS_TOKEN not set"))?;
    let client = reqwest::Client::new();
    let mut request = client
        .request(method, format!("{}/api/{}", url.trim_end_matches('/'), path))
        .header("Authorization", format!("Bearer {}", token));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| crate::error::AgentError::tool(format!("Home Assistant: {}", e)))?;
    let status = response.status();
    let value: serde_json::Value = response
        .json()
        .await
        .unwrap_or_else(|_| json!({"raw": "no json body"}));
    let _ = ctx;
    if !status.is_success() {
        return Err(crate::error::AgentError::tool(format!(
            "Home Assistant returned {}: {}",
            status, value
        )));
    }
    Ok(value)
}

fn register_homeassistant(registry: &mut ToolRegistry) {
    registry.register(
        tool("ha_list_entities")
            .description("List Home Assistant entity ids, optionally filtered by domain (light, switch, sensor...).")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "domain": {"type": "string", "description": "Optional domain filter, e.g. 'light'"}
                },
                "required": []
            }))
            .handler(|args, ctx| async move {
                let domain = args.get("domain").and_then(|v| v.as_str()).map(String::from);
                match hass_request(&ctx, reqwest::Method::GET, "states", None).await {
                    Ok(states) => {
                        let entities: Vec<serde_json::Value> = states
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter(|state| {
                                        domain.as_ref().map(|d| {
                                            state.pointer("/entity_id").and_then(|v| v.as_str())
                                                .map(|id| id.starts_with(&format!("{}.", d)))
                                                .unwrap_or(false)
                                        }).unwrap_or(true)
                                    })
                                    .map(|state| json!({
                                        "entity_id": state.pointer("/entity_id").and_then(|v| v.as_str()),
                                        "state": state.pointer("/state").and_then(|v| v.as_str()),
                                        "friendly_name": state.pointer("/attributes/friendly_name").and_then(|v| v.as_str()),
                                    }))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Ok(json!({"success": true, "entities": entities, "count": entities.len()}))
                    }
                    Err(e) => Ok(json!({"success": false, "error": e.to_string()})),
                }
            })
            .toolset("homeassistant")
            .emoji("🏠")
            .check_fn(hass_configured)
            .build()
            .expect("ha_list_entities builds"),
    );

    registry.register(
        tool("ha_get_state")
            .description("Get the current state and attributes of one Home Assistant entity.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "entity_id": {"type": "string", "description": "Entity id, e.g. 'light.kitchen'"}
                },
                "required": ["entity_id"]
            }))
            .handler(|args, ctx| async move {
                let Some(entity_id) = args.get("entity_id").and_then(|v| v.as_str()) else {
                    return Ok(json!({"success": false, "error": "ha_get_state: 'entity_id' is required"}));
                };
                match hass_request(&ctx, reqwest::Method::GET, &format!("states/{}", entity_id), None).await {
                    Ok(state) => Ok(json!({"success": true, "entity": state})),
                    Err(e) => Ok(json!({"success": false, "error": e.to_string()})),
                }
            })
            .toolset("homeassistant")
            .emoji("🏠")
            .check_fn(hass_configured)
            .build()
            .expect("ha_get_state builds"),
    );

    registry.register(
        tool("ha_list_services")
            .description("List available Home Assistant services, optionally filtered by domain.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "domain": {"type": "string", "description": "Optional domain filter, e.g. 'light'"}
                },
                "required": []
            }))
            .handler(|args, ctx| async move {
                let domain = args.get("domain").and_then(|v| v.as_str()).map(String::from);
                match hass_request(&ctx, reqwest::Method::GET, "services", None).await {
                    Ok(services) => {
                        let filtered: Vec<serde_json::Value> = services
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter(|svc| {
                                        domain.as_ref().map(|d| {
                                            svc.get("domain").and_then(|v| v.as_str()) == Some(d.as_str())
                                        }).unwrap_or(true)
                                    })
                                    .map(|svc| json!({
                                        "domain": svc.get("domain").and_then(|v| v.as_str()),
                                        "services": svc.get("services").and_then(|v| v.as_object()).map(|o| o.keys().collect::<Vec<_>>()),
                                    }))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Ok(json!({"success": true, "services": filtered}))
                    }
                    Err(e) => Ok(json!({"success": false, "error": e.to_string()})),
                }
            })
            .toolset("homeassistant")
            .emoji("🏠")
            .check_fn(hass_configured)
            .build()
            .expect("ha_list_services builds"),
    );

    registry.register(
        tool("ha_call_service")
            .description("Call a Home Assistant service (e.g. light/turn_on) with entity and data payload.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "domain": {"type": "string", "description": "Service domain, e.g. 'light'"},
                    "service": {"type": "string", "description": "Service name, e.g. 'turn_on'"},
                    "entity_id": {"type": "string", "description": "Target entity id"},
                    "data": {"type": "object", "description": "Extra service data (brightness, color, ...)"}
                },
                "required": ["domain", "service", "entity_id"]
            }))
            .handler(|args, ctx| async move {
                let domain = args.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("");
                let entity_id = args.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
                if domain.is_empty() || service.is_empty() || entity_id.is_empty() {
                    return Ok(json!({"success": false, "error": "ha_call_service: domain, service and entity_id are required"}));
                }
                let mut data = args.get("data").cloned().and_then(|v| if v.is_object() { Some(v) } else { None }).unwrap_or_else(|| json!({}));
                data["entity_id"] = json!(entity_id);
                match hass_request(&ctx, reqwest::Method::POST, &format!("services/{}/{}", domain, service), Some(data)).await {
                    Ok(result) => Ok(json!({"success": true, "result": result})),
                    Err(e) => Ok(json!({"success": false, "error": e.to_string()})),
                }
            })
            .toolset("homeassistant")
            .dangerous(true)
            .emoji("🏠")
            .check_fn(hass_configured)
            .build()
            .expect("ha_call_service builds"),
    );
}

// ---------------------------------------------------------------------------
// Kanban — local multi-agent coordination board (SQLite-backed)
// ---------------------------------------------------------------------------

fn kanban_store(ctx: &ToolContext) -> Result<rusqlite::Connection> {
    let path = ctx.home.join("kanban.db");
    let conn = rusqlite::Connection::open(&path)
        .map_err(|e| crate::error::AgentError::tool(format!("open kanban db: {}", e)))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS kanban_tasks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'todo',
            assignee TEXT,
            parent_id TEXT,
            created_at REAL NOT NULL,
            updated_at REAL NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            author TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_attachments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            value TEXT NOT NULL,
            created_at REAL NOT NULL
        );",
    )
    .map_err(|e| crate::error::AgentError::tool(format!("kanban schema: {}", e)))?;
    Ok(conn)
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn register_kanban(registry: &mut ToolRegistry) {
    use rusqlite::params;

    registry.register(
        tool("kanban_create")
            .description("Create a kanban task on the local coordination board.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "Task title"},
                    "body": {"type": "string", "description": "Task description"},
                    "assignee": {"type": "string", "description": "Optional assignee (agent name)"}
                },
                "required": ["title"]
            }))
            .handler(|args, ctx| async move {
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
                if title.is_empty() {
                    return Ok(json!({"success": false, "error": "kanban_create: 'title' is required"}));
                }
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let assignee = args.get("assignee").and_then(|v| v.as_str());
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                let id = format!("task-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                if let Err(e) = conn.execute(
                    "INSERT INTO kanban_tasks (id, title, body, status, assignee, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'todo', ?4, ?5, ?5)",
                    params![id, title, body, assignee, now()],
                ) {
                    return Ok(json!({"success": false, "error": e.to_string()}));
                }
                Ok(json!({"success": true, "task_id": id}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_create builds"),
    );

    registry.register(
        tool("kanban_list")
            .description("List kanban tasks, optionally filtered by status (todo/doing/blocked/done).")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["todo", "doing", "blocked", "done"], "description": "Optional status filter"}
                },
                "required": []
            }))
            .handler(|args, ctx| async move {
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                let status = args.get("status").and_then(|v| v.as_str());
                let (sql, params): (&str, Vec<String>) = if let Some(status) = status {
                    ("SELECT id, title, status, assignee FROM kanban_tasks WHERE status = ?1 ORDER BY updated_at DESC", vec![status.to_string()])
                } else {
                    ("SELECT id, title, status, assignee FROM kanban_tasks ORDER BY updated_at DESC", vec![])
                };
                let mut stmt = match conn.prepare(sql) {
                    Ok(s) => s,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                let rows: Vec<serde_json::Value> = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                        Ok(json!({
                            "id": row.get::<_, String>(0)?,
                            "title": row.get::<_, String>(1)?,
                            "status": row.get::<_, String>(2)?,
                            "assignee": row.get::<_, Option<String>>(3)?,
                        }))
                    })
                    .map_err(|e| crate::error::AgentError::tool(e.to_string()))?
                    .flatten()
                    .collect();
                Ok(json!({"success": true, "tasks": rows, "count": rows.len()}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_list builds"),
    );

    let set_status = |name: &str, status: &'static str, desc: &str| {
        tool(name)
            .description(desc)
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task id to update"},
                    "comment": {"type": "string", "description": "Optional comment explaining the transition"}
                },
                "required": ["task_id"]
            }))
            .handler(move |args, ctx| {
                let status = status;
                async move {
                    let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) else {
                        return Ok(json!({"success": false, "error": "task_id is required"}));
                    };
                    let conn = match kanban_store(&ctx) {
                        Ok(c) => c,
                        Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                    };
                    match conn.execute(
                        "UPDATE kanban_tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
                        params![task_id, status, now()],
                    ) {
                        Ok(0) => return Ok(json!({"success": false, "error": format!("task '{}' not found", task_id)})),
                        Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                        _ => {}
                    }
                    if let Some(comment) = args.get("comment").and_then(|v| v.as_str()) {
                        conn.execute(
                            "INSERT INTO kanban_comments (task_id, author, body, created_at) VALUES (?1, 'agent', ?2, ?3)",
                            params![task_id, comment, now()],
                        )
                        .ok();
                    }
                    Ok(json!({"success": true, "task_id": task_id, "status": status}))
                }
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .unwrap_or_else(|_| panic!("{} builds", name))
    };

    registry.register(set_status("kanban_complete", "done", "Mark a kanban task done."));
    registry.register(set_status("kanban_block", "blocked", "Mark a kanban task blocked (add a comment saying why)."));
    registry.register(set_status("kanban_unblock", "todo", "Unblock a kanban task (moves it back to todo)."));

    registry.register(
        tool("kanban_show")
            .description("Show one kanban task with comments and attachments.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task id"}
                },
                "required": ["task_id"]
            }))
            .handler(|args, ctx| async move {
                let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) else {
                    return Ok(json!({"success": false, "error": "task_id is required"}));
                };
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                let task = conn
                    .query_row(
                        "SELECT id, title, body, status, assignee FROM kanban_tasks WHERE id = ?1",
                        params![task_id],
                        |row| {
                            Ok(json!({
                                "id": row.get::<_, String>(0)?,
                                "title": row.get::<_, String>(1)?,
                                "body": row.get::<_, String>(2)?,
                                "status": row.get::<_, String>(3)?,
                                "assignee": row.get::<_, Option<String>>(4)?,
                            }))
                        },
                    )
                    .optional()
                    .map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                let Some(task) = task else {
                    return Ok(json!({"success": false, "error": format!("task '{}' not found", task_id)}));
                };
                let mut stmt = conn.prepare("SELECT author, body FROM kanban_comments WHERE task_id = ?1 ORDER BY id").unwrap();
                let comments: Vec<serde_json::Value> = stmt
                    .query_map(params![task_id], |row| {
                        Ok(json!({"author": row.get::<_, String>(0)?, "body": row.get::<_, String>(1)?}))
                    })
                    .unwrap()
                    .flatten()
                    .collect();
                let mut stmt = conn.prepare("SELECT kind, value FROM kanban_attachments WHERE task_id = ?1 ORDER BY id").unwrap();
                let attachments: Vec<serde_json::Value> = stmt
                    .query_map(params![task_id], |row| {
                        Ok(json!({"kind": row.get::<_, String>(0)?, "value": row.get::<_, String>(1)?}))
                    })
                    .unwrap()
                    .flatten()
                    .collect();
                Ok(json!({"success": true, "task": task, "comments": comments, "attachments": attachments}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_show builds"),
    );

    registry.register(
        tool("kanban_comment")
            .description("Add a comment to a kanban task.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task id"},
                    "body": {"type": "string", "description": "Comment text"}
                },
                "required": ["task_id", "body"]
            }))
            .handler(|args, ctx| async move {
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                if task_id.is_empty() || body.is_empty() {
                    return Ok(json!({"success": false, "error": "task_id and body are required"}));
                }
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                conn.execute(
                    "INSERT INTO kanban_comments (task_id, author, body, created_at) VALUES (?1, 'agent', ?2, ?3)",
                    params![task_id, body, now()],
                )
                .map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                Ok(json!({"success": true}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_comment builds"),
    );

    registry.register(
        tool("kanban_heartbeat")
            .description("Signal progress on a kanban task (updates its timestamp + optional progress comment).")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task id"},
                    "progress": {"type": "string", "description": "Short progress note"}
                },
                "required": ["task_id"]
            }))
            .handler(|args, ctx| async move {
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                if task_id.is_empty() {
                    return Ok(json!({"success": false, "error": "task_id is required"}));
                }
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                conn.execute("UPDATE kanban_tasks SET status='doing', updated_at=?2 WHERE id=?1", params![task_id, now()]).ok();
                if let Some(progress) = args.get("progress").and_then(|v| v.as_str()) {
                    conn.execute(
                        "INSERT INTO kanban_comments (task_id, author, body, created_at) VALUES (?1, 'agent', ?2, ?3)",
                        params![task_id, format!("[heartbeat] {}", progress), now()],
                    )
                    .ok();
                }
                Ok(json!({"success": true, "task_id": task_id}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_heartbeat builds"),
    );

    registry.register(
        tool("kanban_link")
            .description("Link a kanban task to a parent task (subtask relationship).")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Child task id"},
                    "parent_id": {"type": "string", "description": "Parent task id"}
                },
                "required": ["task_id", "parent_id"]
            }))
            .handler(|args, ctx| async move {
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                let parent_id = args.get("parent_id").and_then(|v| v.as_str()).unwrap_or("");
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                conn.execute("UPDATE kanban_tasks SET parent_id=?2, updated_at=?3 WHERE id=?1", params![task_id, parent_id, now()])
                    .map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                Ok(json!({"success": true}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_link builds"),
    );

    let attach = |name: &'static str, kind: &'static str, value_field: &'static str, desc: &'static str| {
        tool(name)
            .description(desc)
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task id"},
                    value_field: {"type": "string", "description": "Attachment value"}
                },
                "required": ["task_id", value_field]
            }))
            .handler(move |args, ctx| {
                let kind = kind;
                let value_field = value_field;
                async move {
                    let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                    let value = args.get(&value_field).and_then(|v| v.as_str()).unwrap_or("");
                    if task_id.is_empty() || value.is_empty() {
                        return Ok(json!({"success": false, "error": format!("task_id and {} are required", value_field)}));
                    }
                    let conn = match kanban_store(&ctx) {
                        Ok(c) => c,
                        Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                    };
                    conn.execute(
                        "INSERT INTO kanban_attachments (task_id, kind, value, created_at) VALUES (?1, ?2, ?3, ?4)",
                        params![task_id, kind, value, now()],
                    )
                    .map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                    Ok(json!({"success": true}))
                }
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .unwrap_or_else(|_| panic!("{} builds", name))
    };
    registry.register(attach("kanban_attach", "file", "path", "Attach a local file path to a kanban task."));
    registry.register(attach("kanban_attach_url", "url", "url", "Attach a URL to a kanban task."));

    registry.register(
        tool("kanban_attachments")
            .description("List attachments of a kanban task.")
            .parameters(json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task id"}
                },
                "required": ["task_id"]
            }))
            .handler(|args, ctx| async move {
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
                let conn = match kanban_store(&ctx) {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
                };
                let mut stmt = conn.prepare("SELECT kind, value FROM kanban_attachments WHERE task_id = ?1 ORDER BY id").unwrap();
                let attachments: Vec<serde_json::Value> = stmt
                    .query_map(params![task_id], |row| {
                        Ok(json!({"kind": row.get::<_, String>(0)?, "value": row.get::<_, String>(1)?}))
                    })
                    .unwrap()
                    .flatten()
                    .collect();
                Ok(json!({"success": true, "attachments": attachments}))
            })
            .toolset("kanban")
            .emoji("📋")
            .build()
            .expect("kanban_attachments builds"),
    );
}

use rusqlite::OptionalExtension;

// ---------------------------------------------------------------------------
// Browser tools — registered with faithful schemas, gated on CDP endpoint
// ---------------------------------------------------------------------------

fn browser_configured() -> ToolAvailability {
    if crate::config::get_env_value("ULNCLAW_BROWSER_CDP").is_some() {
        ToolAvailability::available()
    } else {
        ToolAvailability::unavailable(
            "no browser backend configured (set ULNCLAW_BROWSER_CDP to a Chrome DevTools endpoint)",
        )
    }
}

fn register_browser(registry: &mut ToolRegistry) {
    let specs: [(&str, &str, serde_json::Value); 12] = [
        ("browser_navigate", "Navigate the browser to a URL.", json!({
            "type": "object",
            "properties": {"url": {"type": "string", "description": "URL to open"}},
            "required": ["url"]
        })),
        ("browser_snapshot", "Capture an accessibility snapshot of the current page (element refs for interaction).", json!({
            "type": "object", "properties": {}, "required": []
        })),
        ("browser_click", "Click an element from the last snapshot.", json!({
            "type": "object",
            "properties": {"element": {"type": "string", "description": "Element ref or selector"}},
            "required": ["element"]
        })),
        ("browser_type", "Type text into an input element.", json!({
            "type": "object",
            "properties": {
                "element": {"type": "string", "description": "Element ref or selector"},
                "text": {"type": "string", "description": "Text to type"}
            },
            "required": ["element", "text"]
        })),
        ("browser_scroll", "Scroll the page (up/down, pixels).", json!({
            "type": "object",
            "properties": {
                "direction": {"type": "string", "enum": ["up", "down"], "default": "down"},
                "pixels": {"type": "integer", "default": 800}
            },
            "required": []
        })),
        ("browser_back", "Navigate back in history.", json!({"type": "object", "properties": {}, "required": []})),
        ("browser_press", "Press a keyboard key (Enter, Tab, Escape...).", json!({
            "type": "object",
            "properties": {"key": {"type": "string", "description": "Key name"}},
            "required": ["key"]
        })),
        ("browser_get_images", "List images on the current page with URLs.", json!({"type": "object", "properties": {}, "required": []})),
        ("browser_vision", "Screenshot the page and analyze it with the vision model.", json!({
            "type": "object",
            "properties": {"prompt": {"type": "string", "description": "What to look for in the screenshot"}},
            "required": []
        })),
        ("browser_console", "Evaluate JavaScript in the page console and return the result.", json!({
            "type": "object",
            "properties": {"expression": {"type": "string", "description": "JS expression"}},
            "required": ["expression"]
        })),
        ("browser_cdp", "Send a raw Chrome DevTools Protocol command.", json!({
            "type": "object",
            "properties": {
                "method": {"type": "string", "description": "CDP method, e.g. Page.captureScreenshot"},
                "params": {"type": "object", "description": "CDP params"}
            },
            "required": ["method"]
        })),
        ("browser_dialog", "Handle a JavaScript dialog (accept/dismiss).", json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["accept", "dismiss"], "default": "accept"},
                "prompt_text": {"type": "string", "description": "Text for prompt dialogs"}
            },
            "required": []
        })),
    ];

    for (name, description, parameters) in specs {
        registry.register(
            tool(name)
                .description(description)
                .parameters(parameters)
                .handler(move |args, ctx| {
                    let tool_name = name.to_string();
                    async move {
                        use crate::browser::with_session;
                        match tool_name.as_str() {
                            "browser_navigate" => {
                                let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                                if url.is_empty() {
                                    return Ok(json!({"success": false, "error": "url is required"}));
                                }
                                with_session(move |session| async move {
                                    session.navigate(&url).await?;
                                    let info = session.page_info().await.unwrap_or(json!({}));
                                    Ok(json!({
                                        "success": true,
                                        "url": info.get("url").cloned().unwrap_or(json!("")),
                                        "title": info.get("title").cloned().unwrap_or(json!("")),
                                        "hint": "call browser_snapshot to see interactive elements"
                                    }))
                                })
                                .await
                            }
                            "browser_snapshot" => with_session(|session| async move {
                                let (text, refs) = session.snapshot().await?;
                                Ok(json!({
                                    "success": true,
                                    "snapshot": text,
                                    "elements": refs.len(),
                                    "hint": "interact via browser_click/browser_type with element refs like \"3\""
                                }))
                            })
                            .await,
                            "browser_click" => {
                                let element = args.get("element").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                                if element.is_empty() {
                                    return Ok(json!({"success": false, "error": "element is required"}));
                                }
                                with_session(move |session| async move {
                                    let result = session.click(&element).await?;
                                    Ok(json!({"success": true, "clicked": result.get("clicked").cloned()}))
                                })
                                .await
                            }
                            "browser_type" => {
                                let element = args.get("element").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                if element.is_empty() {
                                    return Ok(json!({"success": false, "error": "element is required"}));
                                }
                                with_session(move |session| async move {
                                    session.type_text(&element, &text).await?;
                                    Ok(json!({"success": true, "typed_chars": text.len(), "into": element}))
                                })
                                .await
                            }
                            "browser_scroll" => {
                                let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("down").to_string();
                                let pixels = args.get("pixels").and_then(|v| v.as_u64()).unwrap_or(800);
                                with_session(move |session| async move {
                                    session.scroll(&direction, pixels).await?;
                                    Ok(json!({"success": true, "direction": direction, "pixels": pixels}))
                                })
                                .await
                            }
                            "browser_back" => with_session(|session| async move {
                                session.go_back().await?;
                                let info = session.page_info().await.unwrap_or(json!({}));
                                Ok(json!({"success": true, "url": info.get("url").cloned()}))
                            })
                            .await,
                            "browser_press" => {
                                let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                                if key.is_empty() {
                                    return Ok(json!({"success": false, "error": "key is required"}));
                                }
                                with_session(move |session| async move {
                                    session.press(&key).await?;
                                    Ok(json!({"success": true, "pressed": key}))
                                })
                                .await
                            }
                            "browser_get_images" => with_session(|session| async move {
                                let images = session.get_images().await?;
                                Ok(json!({"success": true, "images": images}))
                            })
                            .await,
                            "browser_vision" => {
                                let prompt = args
                                    .get("prompt")
                                    .and_then(|v| v.as_str())
                                    .filter(|p| !p.trim().is_empty())
                                    .unwrap_or("Describe what is visible in this screenshot.")
                                    .to_string();
                                let Some(provider) = ctx.provider.clone() else {
                                    return Ok(json!({"success": false, "error": "no vision-capable provider configured"}));
                                };
                                with_session(move |session| async move {
                                    let png = session.screenshot().await?;
                                    let image_url = format!("data:image/png;base64,{}", png);
                                    match provider.analyze_image(&prompt, &image_url).await {
                                        Ok(analysis) => Ok(json!({"success": true, "analysis": analysis})),
                                        Err(e) => Ok(json!({"success": false, "error": format!("vision provider: {}", e)})),
                                    }
                                })
                                .await
                            }
                            "browser_console" => {
                                let expression = args.get("expression").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                if expression.is_empty() {
                                    return Ok(json!({"success": false, "error": "expression is required"}));
                                }
                                with_session(move |session| async move {
                                    let value = session.evaluate(&expression, None).await?;
                                    Ok(json!({"success": true, "result": value}))
                                })
                                .await
                            }
                            "browser_cdp" => {
                                let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                                let params = args.get("params").cloned().unwrap_or(json!({}));
                                if method.is_empty() {
                                    return Ok(json!({"success": false, "error": "method is required"}));
                                }
                                with_session(move |session| async move {
                                    let result = session.client().call(&method, params).await?;
                                    Ok(json!({"success": true, "result": result}))
                                })
                                .await
                            }
                            "browser_dialog" => {
                                let accept = args.get("action").and_then(|v| v.as_str()).unwrap_or("accept") != "dismiss";
                                let prompt_text = args.get("prompt_text").and_then(|v| v.as_str()).map(String::from);
                                with_session(move |session| async move {
                                    let result = session.handle_dialog(accept, prompt_text.as_deref()).await?;
                                    Ok(json!({"success": true, "dialog": result}))
                                })
                                .await
                            }
                            _ => Ok(json!({"success": false, "error": format!("unknown browser tool: {}", tool_name)})),
                        }
                    }
                })
                .toolset("browser")
                .emoji("🌐")
                .check_fn(browser_configured)
                .build()
                .unwrap_or_else(|_| panic!("{} builds", name)),
        );
    }
}

// ---------------------------------------------------------------------------
// Gated stubs: computer_use + messaging platforms
// ---------------------------------------------------------------------------

fn register_gated_stubs(registry: &mut ToolRegistry) {
    // computer_use — requires a CUA driver (hermes: gated on cua-driver).
    registry.register(
        tool("computer_use")
            .description(
                "Control the desktop: take screenshots, move the mouse, click, and type \
                 (requires a computer-use driver).",
            )
            .parameters(json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["screenshot", "click", "type", "key", "scroll"], "description": "Action to perform"},
                    "x": {"type": "integer", "description": "X coordinate for click"},
                    "y": {"type": "integer", "description": "Y coordinate for click"},
                    "text": {"type": "string", "description": "Text for type action"},
                    "key": {"type": "string", "description": "Key for key action"}
                },
                "required": ["action"]
            }))
            .handler(|_args, _ctx| async move {
                Ok(json!({
                    "success": false,
                    "error": "computer_use: no computer-use driver is available in this build."
                }))
            })
            .toolset("computer_use")
            .dangerous(true)
            .emoji("🖥️")
            .check_fn(|| ToolAvailability::unavailable("computer-use driver not installed"))
            .build()
            .expect("computer_use builds"),
    );

    // Messaging platforms — faithful tool surface, gated on credentials.
    let platforms: [(&str, &str, &str, &str); 4] = [
        ("discord", "discord", "Send and manage Discord messages in servers the bot has joined.", "DISCORD_BOT_TOKEN"),
        ("discord_admin", "discord_admin", "Discord server administration (channels, roles, moderation).", "DISCORD_BOT_TOKEN"),
        ("feishu_doc_read", "feishu_doc", "Read a Feishu/Lark document as markdown.", "FEISHU_APP_ID"),
        ("spotify_playback", "spotify", "Control Spotify playback (play/pause/skip/current track).", "SPOTIFY_CLIENT_ID"),
    ];
    for (name, toolset, description, env_var) in platforms {
        let env_var = env_var.to_string();
        registry.register(
            tool(name)
                .description(description)
                .parameters(json!({"type": "object", "properties": {}, "required": []}))
                .handler(move |_args, _ctx| {
                    let name = name.to_string();
                    async move {
                        Ok(json!({
                            "success": false,
                            "error": format!("{}: platform backend is not implemented in this build.", name)
                        }))
                    }
                })
                .toolset(toolset)
                .emoji("🔌")
                .check_fn(move || {
                    if crate::config::get_env_value(&env_var).is_some() {
                        ToolAvailability::available()
                    } else {
                        ToolAvailability::unavailable(format!("{} not set", env_var))
                    }
                })
                .build()
                .unwrap_or_else(|_| panic!("{} builds", name)),
        );
    }
}
