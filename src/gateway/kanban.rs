//! Kanban board HTTP API — the desktop widget and any external client talk
//! to these endpoints; they share the same `KanbanStore` engine (and the
//! same `kanban.db`) as the `ulnclaw kanban` CLI and the agent-side
//! `kanban_*` tools (hermes parity: one board, three surfaces).

use axum::{
    extract::{Path, Query},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::kanban::{KanbanStore, NewTask, DEFAULT_CLAIM_TTL_SECS};

fn store() -> Result<KanbanStore, Response> {
    KanbanStore::open_default().map_err(|e| super::server_error(&e.to_string()))
}

/// Resolve a task id or unique prefix; 404s when unknown.
fn resolve(store: &KanbanStore, id: &str) -> Result<String, Response> {
    if let Ok(Some(task)) = store.get_task(id) {
        return Ok(task.id);
    }
    match store.resolve_task_id(id) {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(super::not_found(&format!("task {id} not found"))),
        Err(e) => Err(super::server_error(&e.to_string())),
    }
}

fn task_json(store: &KanbanStore, task: &crate::kanban::Task) -> Value {
    let parents = store.parents_of(&task.id).unwrap_or_default();
    let children = store.children_of(&task.id).unwrap_or_default();
    json!({
        "id": task.id,
        "board": task.board,
        "title": task.title,
        "body": task.body,
        "assignee": task.assignee,
        "status": task.status,
        "priority": task.priority,
        "created_by": task.created_by,
        "created_at": task.created_at,
        "started_at": task.started_at,
        "completed_at": task.completed_at,
        "result": task.result,
        "claim_lock": task.claim_lock,
        "claim_expires": task.claim_expires,
        "last_heartbeat_at": task.last_heartbeat_at,
        "skills": task.skills,
        "max_runtime_seconds": task.max_runtime_seconds,
        "idempotency_key": task.idempotency_key,
        "parents": parents,
        "children": children,
    })
}

/// `GET /api/kanban/boards` — boards with open/total task counts.
pub async fn list_boards() -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let boards = match store.list_boards() {
        Ok(b) => b,
        Err(e) => return super::server_error(&e.to_string()),
    };
    let counts = store.board_task_counts().unwrap_or_default();
    let current = store.current_board().unwrap_or_default();
    let rows: Vec<Value> = boards
        .iter()
        .map(|b| {
            let (open, total) = counts
                .iter()
                .find(|(slug, _, _)| slug == &b.slug)
                .map(|(_, open, total)| (*open, *total))
                .unwrap_or((0, 0));
            json!({
                "slug": b.slug,
                "name": b.name,
                "default_workdir": b.default_workdir,
                "created_at": b.created_at,
                "current": b.slug == current,
                "open_tasks": open,
                "total_tasks": total,
            })
        })
        .collect();
    Json(json!({"object": "ulnclaw.kanban.board.list", "boards": rows})).into_response()
}

#[derive(Deserialize)]
pub struct CreateBoardBody {
    pub slug: String,
    pub name: Option<String>,
    pub default_workdir: Option<String>,
}

/// `POST /api/kanban/boards` — create a board.
pub async fn create_board(Json(body): Json<CreateBoardBody>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.create_board(&body.slug, body.name.as_deref(), body.default_workdir.as_deref()) {
        Ok(board) => Json(json!({"object": "ulnclaw.kanban.board", "board": board})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

/// `POST /api/kanban/boards/:slug/switch` — set the current board.
pub async fn switch_board(Path(slug): Path<String>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.switch_board(&slug) {
        Ok(()) => Json(json!({"ok": true, "current_board": slug})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

#[derive(Deserialize, Default)]
pub struct ListTasksQuery {
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub board: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /api/kanban/tasks` — tasks on a board (default: current board).
pub async fn list_tasks(Query(query): Query<ListTasksQuery>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let limit = query.limit.unwrap_or(200).max(1);
    let tasks = match store.list_tasks(
        query.board.as_deref(),
        query.status.as_deref().filter(|s| !s.is_empty()),
        query.assignee.as_deref().filter(|s| !s.is_empty()),
        limit,
    ) {
        Ok(t) => t,
        Err(e) => return super::server_error(&e.to_string()),
    };
    let rows: Vec<Value> = tasks.iter().map(|t| task_json(&store, t)).collect();
    Json(json!({
        "object": "ulnclaw.kanban.task.list",
        "board": store.current_board().unwrap_or_default(),
        "count": rows.len(),
        "tasks": rows,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct CreateTaskBody {
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub parents: Option<Vec<String>>,
    #[serde(default)]
    pub priority: Option<i64>,
    /// Skills force-loaded into the dispatcher worker prompt.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// Per-attempt runtime cap in seconds (dispatcher-enforced).
    #[serde(default)]
    pub max_runtime_seconds: Option<i64>,
    /// Dedup key — an existing non-archived task with the key is returned.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Park in the triage column for the specifier/decomposer.
    #[serde(default)]
    pub triage: Option<bool>,
    /// Circuit-breaker threshold: block on the Nth failed attempt
    /// (hermes max_retries; must be >= 1).
    #[serde(default)]
    pub max_retries: Option<i64>,
    /// Workspace: `scratch` | `worktree` | `worktree:<path>` |
    /// `dir:<path>` (hermes create --workspace).
    #[serde(default)]
    pub workspace: Option<String>,
    /// Worktree branch name (hermes create --branch; requires a
    /// worktree workspace).
    #[serde(default)]
    pub branch: Option<String>,
    /// Creator session id for wake routing (hermes create_task
    /// session_id); the notifier wakes this session on terminal events.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// `POST /api/kanban/tasks` — create a task on the current board.
pub async fn create_task(Json(body): Json<CreateTaskBody>) -> Response {
    if body.title.trim().is_empty() {
        return super::bad_request("title is required", None);
    }
    if let Some(max_retries) = body.max_retries {
        if max_retries < 1 {
            return super::bad_request(
                &format!(
                    "max_retries must be >= 1 (got {max_retries}); use 1 to trip on the first failure"
                ),
                None,
            );
        }
    }
    // Workspace flags (hermes _cmd_create): parse + validate, then
    // fall back to the [kanban] worktrees default when unset.
    let (mut workspace_kind, workspace_path) = match body.workspace.as_deref() {
        Some(raw) => match crate::kanban::parse_workspace_flag(raw) {
            Ok(parsed) => parsed,
            Err(e) => return super::bad_request(&e, None),
        },
        None => ("scratch".to_string(), None),
    };
    let branch_name = match body.branch.as_deref() {
        Some(raw) => match crate::kanban::parse_branch_flag(raw) {
            Ok(branch) => Some(branch),
            Err(e) => return super::bad_request(&e, None),
        },
        None => None,
    };
    if branch_name.is_some() && workspace_kind != "worktree" {
        return super::bad_request(
            "--branch is only valid with --workspace worktree",
            None,
        );
    }
    if body.workspace.is_none()
        && crate::config::UlncLawConfig::load(None)
            .map(|c| c.kanban.worktrees)
            .unwrap_or(false)
    {
        workspace_kind = "worktree".to_string();
    }
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let task = match store.create_task(&NewTask {
        title: body.title.trim().to_string(),
        body: body.body.unwrap_or_default(),
        assignee: body.assignee.filter(|a| !a.trim().is_empty()),
        priority: body.priority.unwrap_or(0),
        tenant: None,
        model: None,
        created_by: "gateway".to_string(),
        skills: body.skills.filter(|s| !s.is_empty()),
        max_runtime_seconds: body.max_runtime_seconds,
        idempotency_key: body.idempotency_key.filter(|k| !k.trim().is_empty()),
        triage: body.triage.unwrap_or(false),
        max_retries: body.max_retries,
        workspace_kind: Some(workspace_kind),
        workspace_path,
        branch_name,
        session_id: body.session_id.filter(|id| !id.trim().is_empty()),
    }) {
        Ok(t) => t,
        Err(e) => return super::bad_request(&e.to_string(), None),
    };
    for parent in body.parents.unwrap_or_default() {
        if let Ok(Some(parent_id)) = store.resolve_task_id(&parent) {
            store.link_tasks(&parent_id, &task.id).ok();
        }
    }
    Json(json!({
        "object": "ulnclaw.kanban.task",
        "task": task_json(&store, &task),
    }))
    .into_response()
}

/// `GET /api/kanban/tasks/:id` — task with comments + attachments.
pub async fn get_task(Path(id): Path<String>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let id = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let task = match store.get_task(&id) {
        Ok(Some(t)) => t,
        Ok(None) => return super::not_found(&format!("task {id} not found")),
        Err(e) => return super::server_error(&e.to_string()),
    };
    let comments: Vec<Value> = store
        .comments(&id)
        .unwrap_or_default()
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "author": c.author,
                "body": c.body,
                "created_at": c.created_at,
            })
        })
        .collect();
    let attachments: Vec<Value> = store
        .attachments(&id)
        .unwrap_or_default()
        .iter()
        .map(|(kind, value)| json!({"kind": kind, "value": value}))
        .collect();
    let events: Vec<Value> = store
        .events(&id)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            json!({
                "kind": e.kind,
                "payload": e.payload,
                "created_at": e.created_at,
            })
        })
        .collect();
    Json(json!({
        "object": "ulnclaw.kanban.task",
        "task": task_json(&store, &task),
        "comments": comments,
        "attachments": attachments,
        "events": events,
    }))
    .into_response()
}

#[derive(Deserialize, Default)]
pub struct CompleteBody {
    #[serde(default)]
    pub result: Option<String>,
    /// Structured handoff summary for downstream tasks (hermes
    /// complete summary=).
    #[serde(default)]
    pub summary: Option<String>,
    /// JSON object of structured facts stored on the closing run
    /// (hermes complete metadata=).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Deliverable file paths; scratch-workspace files are staged into
    /// the attachments dir (hermes kanban_complete artifacts=).
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
    /// Task ids this worker claims to have created; verified before
    /// completion, phantom ids block it (hermes created_cards).
    #[serde(default)]
    pub created_cards: Option<Vec<String>>,
}

/// `POST /api/kanban/tasks/:id/complete` — mark done.
pub async fn complete_task(Path(id): Path<String>, Json(body): Json<CompleteBody>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let id = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let metadata = match body.metadata.as_ref() {
        Some(value) if !value.is_object() => {
            return super::bad_request("metadata must be a JSON object", None)
        }
        other => other,
    };
    let artifacts: Vec<String> = body
        .artifacts
        .unwrap_or_default()
        .into_iter()
        .filter_map(|raw| {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        })
        .collect();
    let created_cards: Vec<String> = body
        .created_cards
        .unwrap_or_default()
        .into_iter()
        .filter_map(|raw| {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        })
        .collect();
    let home = crate::config::ulnclaw_home();
    match store.complete_task_with_artifacts(
        &home,
        &id,
        body.result.as_deref().filter(|s| !s.trim().is_empty()),
        body.summary.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        metadata,
        &artifacts,
        &created_cards,
    ) {
        Ok(task) => Json(json!({"object": "ulnclaw.kanban.task", "task": task_json(&store, &task)})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

#[derive(Deserialize)]
pub struct BlockBody {
    pub reason: String,
    /// Typed block kind (hermes block --kind): dependency |
    /// needs_input | capability | transient.
    #[serde(default)]
    pub kind: Option<String>,
}

/// `POST /api/kanban/tasks/:id/block` — mark blocked (reason required).
pub async fn block_task(Path(id): Path<String>, Json(body): Json<BlockBody>) -> Response {
    if body.reason.trim().is_empty() {
        return super::bad_request("reason is required", None);
    }
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let id = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    match store.block_task_kind(
        &id,
        body.reason.trim(),
        body.kind
            .as_deref()
            .map(str::trim)
            .filter(|kind| !kind.is_empty()),
    ) {
        Ok(task) => Json(json!({"object": "ulnclaw.kanban.task", "task": task_json(&store, &task)})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

/// `POST /api/kanban/tasks/:id/unblock` — back to todo.
pub async fn unblock_task(Path(id): Path<String>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let id = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    match store.unblock_task(&id) {
        Ok(task) => Json(json!({"object": "ulnclaw.kanban.task", "task": task_json(&store, &task)})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

#[derive(Deserialize)]
pub struct CommentBody {
    pub body: String,
    #[serde(default)]
    pub author: Option<String>,
}

/// `POST /api/kanban/tasks/:id/comment` — add a comment.
pub async fn comment_task(Path(id): Path<String>, Json(body): Json<CommentBody>) -> Response {
    if body.body.trim().is_empty() {
        return super::bad_request("body is required", None);
    }
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let id = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let author = body
        .author
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or("desktop");
    match store.add_comment(&id, author, body.body.trim()) {
        Ok(()) => Json(json!({"ok": true, "task_id": id})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

#[derive(Deserialize)]
pub struct LinkBody {
    pub parent_id: String,
}

/// `POST /api/kanban/tasks/:id/link` — link child → parent.
pub async fn link_task(Path(id): Path<String>, Json(body): Json<LinkBody>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let child = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let parent = match resolve(&store, &body.parent_id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    match store.link_tasks(&parent, &child) {
        Ok(()) => Json(json!({"ok": true, "parent_id": parent, "child_id": child})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}

#[derive(Deserialize, Default)]
pub struct DispatchBody {
    #[serde(default)]
    pub max_spawn: Option<usize>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

/// `POST /api/kanban/dispatch` — one dispatcher tick (hermes
/// `kanban dispatch`): reclaim stale claims, promote parent-done todos,
/// spawn detached `ulnclaw run` workers for ready tasks.
pub async fn dispatch(Json(body): Json<DispatchBody>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let home = crate::config::ulnclaw_home();
    let config = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let max_spawn = body.max_spawn.unwrap_or(config.kanban.max_spawn).max(1);
    let dry_run = body.dry_run.unwrap_or(false);
    let use_worktrees = config.kanban.worktrees;
    let stale_timeout = config.kanban.stale_timeout_seconds;
    let known_profiles: std::collections::HashSet<String> =
        config.profiles.keys().cloned().collect();
    // Spawning child processes is blocking work — keep it off the axum task.
    let outcome = tokio::task::spawn_blocking(move || {
        store.dispatch_once(
            &home,
            use_worktrees,
            |task, workspace| crate::kanban::dispatch_spawn(&home, task, workspace),
            Some(max_spawn),
            dry_run,
            2,
            stale_timeout,
            Some(&known_profiles),
            config.kanban.max_in_progress_per_profile,
        )
    })
    .await;
    match outcome {
        Ok(Ok(result)) => Json(json!({
            "object": "ulnclaw.kanban.dispatch",
            "dry_run": dry_run,
            "reclaimed": result.reclaimed,
            "promoted": result.promoted,
            "spawned": result.spawned,
            "would_spawn": result.would_spawn,
            "respawn_guarded": result.respawn_guarded,
            "skipped_capped": result.skipped_capped,
            "skipped_unassigned": result.skipped_unassigned,
            "skipped_nonspawnable": result.skipped_nonspawnable,
            "skipped_per_profile_capped": result.skipped_per_profile_capped,
            "spawn_failed": result.spawn_failed,
            "auto_blocked": result.auto_blocked,
        }))
        .into_response(),
        Ok(Err(e)) => super::server_error(&e.to_string()),
        Err(e) => super::server_error(&e.to_string()),
    }
}

/// `POST /api/kanban/tasks/:id/claim` — claim a task for work (desktop
/// "start" button); uses the same TTL semantics as the CLI workers.
pub async fn claim_task(Path(id): Path<String>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let id = match resolve(&store, &id) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let claimer = KanbanStore::claimer_id();
    match store.claim_task(&id, &claimer, DEFAULT_CLAIM_TTL_SECS) {
        Ok(task) => Json(json!({"object": "ulnclaw.kanban.task", "task": task_json(&store, &task)})).into_response(),
        Err(e) => super::bad_request(&e.to_string(), None),
    }
}
