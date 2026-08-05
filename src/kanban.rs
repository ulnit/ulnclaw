//! Kanban task engine — port of the hermes `hermes_cli/kanban_db.py`
//! core (boards, tasks, claim locks, comments, events) backing
//! `ulnclaw kanban`.
//!
//! The store lives in `<home>/kanban.db` (hermes keeps a per-profile
//! board). Lifecycle transitions append `task_events` rows and the CLI
//! fires the `kanban_task_claimed` / `kanban_task_completed` /
//! `kanban_task_blocked` hooks that the plugin runtime validates
//! (P95 wired the events; this engine emits them).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{AgentError, Result};

/// Hermes task statuses (`_STATUS_ICONS` keys).
pub const STATUSES: &[&str] = &[
    "todo", "ready", "running", "scheduled", "blocked", "done", "archived",
];

/// Valid task workspace kinds (hermes `VALID_WORKSPACE_KINDS`).
pub const VALID_WORKSPACE_KINDS: &[&str] = &["scratch", "worktree", "dir"];

/// Status glyphs (hermes `_STATUS_ICONS`).
pub fn status_icon(status: &str) -> &'static str {
    match status {
        "triage" => "\u{1FA7A}",   // 🩺
        "todo" => "\u{25FB}",      // ◻
        "ready" => "\u{25B6}",     // ▶
        "running" => "\u{25CF}",   // ●
        "scheduled" => "\u{23F1}", // ⏱
        "blocked" => "\u{2298}",   // ⊘
        "done" => "\u{2713}",      // ✓
        "archived" => "\u{2014}",  // —
        _ => "?",
    }
}

/// Hermes default claim TTL (30 minutes).
pub const DEFAULT_CLAIM_TTL_SECS: i64 = 30 * 60;

/// The default board seeded on first open (hermes `default`).
pub const DEFAULT_BOARD: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub board: String,
    pub title: String,
    pub body: String,
    pub assignee: Option<String>,
    pub status: String,
    pub priority: i64,
    pub created_by: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub tenant: Option<String>,
    pub model: Option<String>,
    pub result: Option<String>,
    pub claim_lock: Option<String>,
    pub claim_expires: Option<i64>,
    pub last_heartbeat_at: Option<i64>,
    /// Pid of the dispatcher-spawned worker (hermes `worker_pid`).
    pub worker_pid: Option<i64>,
    /// Skills force-loaded into the dispatcher worker prompt (hermes
    /// `skills` column, JSON array).
    pub skills: Option<Vec<String>>,
    /// Per-attempt runtime cap in seconds enforced by the dispatcher
    /// (hermes `max_runtime_seconds`).
    pub max_runtime_seconds: Option<i64>,
    /// Creation dedup key (hermes `idempotency_key`).
    pub idempotency_key: Option<String>,
    /// Dispatcher spawn/crash/timeout failures in a row (hermes
    /// `consecutive_failures`); reset by completion and unblock.
    pub consecutive_failures: i64,
    /// Last failure message, capped (hermes `last_failure_error`).
    pub last_failure_error: Option<String>,
    /// Per-task circuit-breaker threshold override (hermes
    /// `max_retries`): block on the Nth failure; NULL = dispatcher
    /// default.
    pub max_retries: Option<i64>,
    /// Workspace kind: `scratch` | `worktree` | `dir` (hermes
    /// `workspace_kind`).
    pub workspace_kind: String,
    /// Requested or resolved workspace directory (hermes
    /// `workspace_path`); the dispatcher persists the resolved path so
    /// retries reuse the same directory.
    pub workspace_path: Option<String>,
    /// Worktree branch (hermes `branch_name`); defaults to `wt/<id>`
    /// at dispatch time when empty.
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub slug: String,
    pub name: String,
    pub default_workdir: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: String,
    pub kind: String,
    pub payload: Value,
    pub created_at: i64,
}

/// One gateway chat subscription to a task's terminal events (hermes
/// `kanban_notify_subs` row). The gateway notifier polls
/// [`KanbanStore::unseen_events_for_sub`] and advances
/// `last_event_id` after delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifySub {
    pub task_id: String,
    pub platform: String,
    pub chat_id: String,
    pub chat_type: Option<String>,
    pub thread_id: String,
    pub user_id: Option<String>,
    pub notifier_profile: Option<String>,
    /// Free-form routing anchor (JSON), e.g. Telegram DM-topic reply ids.
    pub delivery_metadata: Option<Value>,
    pub created_at: i64,
    pub last_event_id: i64,
}

/// One attempt to execute a task — a `task_runs` row (hermes `Run`).
/// Created on claim, closed on complete/block/crash/timeout/spawn-failure/
/// reclaim; retries produce multiple runs per task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub task_id: String,
    pub profile: Option<String>,
    pub step_key: Option<String>,
    /// running | done | blocked | crashed | timed_out | failed |
    /// released | reclaimed
    pub status: String,
    pub claim_lock: Option<String>,
    pub claim_expires: Option<i64>,
    pub worker_pid: Option<i64>,
    pub max_runtime_seconds: Option<i64>,
    pub last_heartbeat_at: Option<i64>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// completed | blocked | crashed | timed_out | spawn_failed |
    /// gave_up | reclaimed (null while running)
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub metadata: Option<Value>,
    pub error: Option<String>,
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let metadata: Option<String> = row.get(14)?;
    Ok(Run {
        id: row.get(0)?,
        task_id: row.get(1)?,
        profile: row.get(2)?,
        step_key: row.get(3)?,
        status: row.get(4)?,
        claim_lock: row.get(5)?,
        claim_expires: row.get(6)?,
        worker_pid: row.get(7)?,
        max_runtime_seconds: row.get(8)?,
        last_heartbeat_at: row.get(9)?,
        started_at: row.get(10)?,
        ended_at: row.get(11)?,
        outcome: row.get(12)?,
        summary: row.get(13)?,
        metadata: metadata
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| serde_json::from_str(s).ok()),
        error: row.get(15)?,
    })
}

/// Arguments for [`KanbanStore::add_notify_sub`].
#[derive(Debug, Clone, Default)]
pub struct NewNotifySub<'a> {
    pub platform: &'a str,
    pub chat_id: &'a str,
    /// dm / group / channel (used by wake routing).
    pub chat_type: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    /// Profile gateway that owns/delivers this subscription.
    pub notifier_profile: Option<&'a str>,
    pub delivery_metadata: Option<Value>,
}

/// Board health snapshot (hermes `board_stats`): per-status +
/// per-assignee counts plus the oldest `ready` age — the clearest
/// staleness signal for a router or HUD.
#[derive(Debug, Clone, Serialize)]
pub struct BoardStats {
    pub by_status: Vec<(String, i64)>,
    pub by_assignee: Vec<(String, Vec<(String, i64)>)>,
    pub oldest_ready_age_seconds: Option<i64>,
    pub now: i64,
}

/// Outcome of one [`KanbanStore::dispatch_once`] tick (hermes
/// `DispatchResult`, scoped).
#[derive(Debug, Clone, Default, Serialize)]
pub struct DispatchResult {
    /// Running tasks whose claim expired and were reset to ready.
    pub reclaimed: Vec<String>,
    /// Todo tasks promoted to ready (all parents done).
    pub promoted: Vec<String>,
    /// Tasks a worker was spawned for, with the worker pid when known.
    pub spawned: Vec<(String, Option<i64>)>,
    /// Tasks that would have spawned (dry-run ticks).
    pub would_spawn: Vec<String>,
    /// Ready tasks skipped because the concurrency cap was reached.
    pub skipped_capped: Vec<String>,
    /// Tasks whose spawn failed (still under the failure limit).
    pub spawn_failed: Vec<String>,
    /// Tasks auto-blocked after `failure_limit` consecutive spawn failures.
    pub auto_blocked: Vec<String>,
    /// Running tasks killed + requeued for exceeding max_runtime_seconds.
    pub reaped: Vec<String>,
    /// Running tasks reclaimed for a stale/missing heartbeat (hermes
    /// `detect_stale_running`).
    pub stale: Vec<String>,
    /// Running tasks whose worker pid died (hermes
    /// `detect_crashed_workers`).
    pub crashed: Vec<String>,
}

/// Worker brief for a dispatcher-spawned task (hermes spawns
/// `hermes chat -q "work kanban task <id>"`; the ulnclaw one-shot is
/// `ulnclaw run`).
/// Worker-context caps (hermes `_CTX_MAX_*`).
const CTX_MAX_PRIOR_ATTEMPTS: usize = 10;
const CTX_MAX_COMMENTS: usize = 30;
const CTX_MAX_FIELD_BYTES: usize = 4 * 1024;
const CTX_MAX_BODY_BYTES: usize = 8 * 1024;
const CTX_MAX_COMMENT_BYTES: usize = 2 * 1024;

/// Truncate to `limit` chars with a visible ellipsis (hermes `_cap`).
fn cap_field(s: &str, limit: usize) -> String {
    let s = s.trim();
    let count = s.chars().count();
    if count <= limit {
        return s.to_string();
    }
    let truncated: String = s.chars().take(limit).collect();
    format!("{truncated}… [truncated, {} chars omitted]", count - limit)
}

/// Coarse human age — "just now" / "18h ago" / "3d ago" (hermes
/// `_relative_age`). Relative ages make an LLM re-verify stale handoffs
/// instead of reading them as current fact.
fn relative_age(ts: i64, now: i64) -> String {
    let delta = now - ts;
    if delta < 60 {
        return "just now".into();
    }
    if delta < 3600 {
        return format!("{}m ago", delta / 60);
    }
    if delta < 86400 {
        return format!("{}h ago", delta / 3600);
    }
    format!("{}d ago", delta / 86400)
}

fn ctx_timestamp(ts: i64, now: i64) -> String {
    let base = chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string());
    let age = relative_age(ts, now);
    if age.is_empty() {
        base
    } else {
        format!("{base}, {age}")
    }
}

pub fn worker_prompt(home: &Path, task: &Task) -> String {
    let mut prompt = format!(
        "You are a kanban worker for task {} ({}). Start by calling kanban_show \
         (task_id defaults to your own task) to read the full brief, then do the \
         work. When finished you MUST call kanban_complete with a result summary; \
         if you cannot proceed, call kanban_block with the reason. Use \
         kanban_heartbeat to report progress on long steps.",
        task.id, task.title
    );
    // Force-loaded skills (hermes passes `--skills <name>` per task; the
    // ulnclaw worker inlines the skill body into its founding prompt).
    if let Some(skills) = task.skills.as_deref().filter(|s| !s.is_empty()) {
        let skills_dir = home.join("skills");
        for name in skills {
            if let Some(message) =
                crate::skills::build_skill_invocation_message(&skills_dir, name, "")
            {
                prompt.push_str("\n\n");
                prompt.push_str(&message);
            }
        }
    }
    prompt
}

/// Dispatch-time spawn: run the detached worker in the workspace the
/// dispatcher resolved for the task (hermes passes the resolved
/// workspace to `_default_spawn`; `None` runs in place).
pub fn dispatch_spawn(
    home: &Path,
    task: &Task,
    workspace: Option<&Path>,
) -> std::result::Result<Option<i64>, String> {
    spawn_worker(home, task, workspace)
}

/// Spawn a detached `ulnclaw run` worker for `task` (hermes
/// `_default_spawn`). The worker gets `ULNCLAW_KANBAN_TASK=<id>` (the
/// worker-context env the kanban_* tools gate on), an optional
/// `--profile <assignee>`, and its output goes to
/// `<home>/kanban/worker-logs/<id>.log`. Returns the worker pid.
pub fn default_spawn(home: &Path, task: &Task) -> std::result::Result<Option<i64>, String> {
    spawn_worker(home, task, None)
}

/// Like [`default_spawn`] but the worker runs in `workdir` (an isolated
/// worktree when the dispatcher prepared one).
pub fn spawn_worker(
    home: &Path,
    task: &Task,
    workdir: Option<&Path>,
) -> std::result::Result<Option<i64>, String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let log_dir = home.join("kanban").join("worker-logs");
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| format!("create {}: {e}", log_dir.display()))?;
    let log_path = log_dir.join(format!("{}.log", task.id));
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| format!("create {}: {e}", log_path.display()))?;
    let err_file = log_file
        .try_clone()
        .map_err(|e| format!("clone log handle: {e}"))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("run").arg(worker_prompt(home, task));
    if let Some(assignee) = task.assignee.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        cmd.arg("--profile").arg(assignee);
    }
    cmd.env("ULNCLAW_KANBAN_TASK", &task.id);
    if let Some(workdir) = workdir {
        cmd.current_dir(workdir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(err_file));
    // New session: the worker survives the dispatcher's process group.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| format!("spawn worker: {e}"))?;
    Ok(Some(i64::from(child.id())))
}

/// `<home>/kanban/worker-logs/<id>.log` (hermes `worker_log_path`).
pub fn worker_log_path(home: &Path, task_id: &str) -> PathBuf {
    home.join("kanban")
        .join("worker-logs")
        .join(format!("{task_id}.log"))
}

/// Read the worker log for `task_id` (hermes `read_worker_log`).
/// Returns `None` when the task has not spawned yet. With `tail_bytes`,
/// only the last N bytes are returned — the partial first line is
/// skipped unless the whole window is a single giant line.
pub fn read_worker_log(home: &Path, task_id: &str, tail_bytes: Option<u64>) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let path = worker_log_path(home, task_id);
    let mut file = std::fs::File::open(&path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut buf: Vec<u8> = Vec::new();
    match tail_bytes {
        None => {
            file.read_to_end(&mut buf).ok()?;
        }
        Some(tail) if size <= tail => {
            file.read_to_end(&mut buf).ok()?;
        }
        Some(tail) => {
            file.seek(SeekFrom::Start(size - tail)).ok()?;
            let probe = size - tail;
            let mut line: Vec<u8> = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                match file.read(&mut byte) {
                    Ok(1) => {
                        line.push(byte[0]);
                        if byte[0] == b'\n' {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            let at_eof = file.stream_position().ok()? >= size;
            if !line.ends_with(b"\n") && at_eof {
                // One giant log line: don't skip anything.
                file.seek(SeekFrom::Start(probe)).ok()?;
            }
            file.read_to_end(&mut buf).ok()?;
        }
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Result of [`repair_db`] (hermes `RepairResult`).
#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    /// ok | repaired | missing | corrupt
    pub status: String,
    pub db_path: PathBuf,
    pub messages: Vec<String>,
    pub post_repair_messages: Vec<String>,
    pub backup_path: Option<PathBuf>,
    pub reindexed: Vec<String>,
}

fn integrity_check(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare("PRAGMA integrity_check") {
        Ok(stmt) => stmt,
        Err(_) => return vec!["integrity_check failed to prepare".into()],
    };
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_else(|_| vec!["integrity_check query failed".into()])
}

fn integrity_ok(messages: &[String]) -> bool {
    messages.len() == 1 && messages[0].trim().eq_ignore_ascii_case("ok")
}

/// Index names iff EVERY integrity message is an index-class error
/// (hermes `_repairable_index_names`); `None` fails closed.
fn repairable_index_names(messages: &[String]) -> Option<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut saw_any = false;
    for raw in messages {
        let message = raw.trim();
        if message.is_empty() {
            continue;
        }
        let index = message
            .strip_prefix("wrong # of entries in index ")
            .or_else(|| {
                // "row <N> missing from index <name>"
                let rest = message.strip_prefix("row ")?;
                let rest = rest.split_once(" missing from index ")?.1;
                Some(rest)
            })
            .or_else(|| {
                message
                    .strip_prefix("row ")
                    .and_then(|r| r.split_once(" missing from index "))
                    .map(|(_, idx)| idx)
            });
        let Some(index) = index.map(str::trim).filter(|n| !n.is_empty()) else {
            return None;
        };
        saw_any = true;
        if !names.contains(&index.to_string()) {
            names.push(index.to_string());
        }
    }
    if !saw_any || names.is_empty() {
        return None;
    }
    Some(names)
}

/// Content-addressed quarantine of a corrupt DB + WAL/SHM sidecars
/// (hermes `_backup_corrupt_db`).
fn backup_corrupt_db(path: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let fingerprint: String = hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    let parent = path.parent()?;
    let backup = parent.join(format!("{}.corrupt-{fingerprint}", path.file_name()?.to_str()?));
    if !backup.exists() {
        std::fs::write(&backup, &bytes).ok()?;
        for suffix in ["-wal", "-shm"] {
            let sidecar = path.with_file_name(format!("{}{suffix}", path.file_name()?.to_str()?));
            if sidecar.exists() {
                let dest = parent.join(format!(
                    "{}.corrupt-{fingerprint}{suffix}",
                    path.file_name()?.to_str()?
                ));
                std::fs::copy(&sidecar, &dest).ok();
            }
        }
    }
    Some(backup)
}

/// Integrity-check a kanban DB and apply the narrow index-REINDEX
/// auto-repair when the damage is entirely index-scoped (hermes
/// `repair_db`). Anything else stays corrupt (fail-closed). Never
/// runs schema init, so it is reachable on exactly the boards that
/// need it.
pub fn repair_db(path: &Path) -> RepairReport {
    let db_path = path.to_path_buf();
    let missing = match std::fs::metadata(&db_path) {
        Ok(meta) => meta.len() == 0,
        Err(_) => true,
    };
    if missing {
        return RepairReport {
            status: "missing".into(),
            db_path,
            messages: Vec::new(),
            post_repair_messages: Vec::new(),
            backup_path: None,
            reindexed: Vec::new(),
        };
    }
    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(e) => {
            let backup_path = backup_corrupt_db(&db_path);
            return RepairReport {
                status: "corrupt".into(),
                db_path,
                messages: vec![format!("sqlite refused to open file: {e}")],
                post_repair_messages: Vec::new(),
                backup_path,
                reindexed: Vec::new(),
            };
        }
    };
    let messages = integrity_check(&conn);
    if integrity_ok(&messages) {
        return RepairReport {
            status: "ok".into(),
            db_path,
            messages,
            post_repair_messages: Vec::new(),
            backup_path: None,
            reindexed: Vec::new(),
        };
    }
    // Quarantine FIRST — identical policy to the connect-time guard.
    let backup_path = backup_corrupt_db(&db_path);
    let Some(index_names) = repairable_index_names(&messages) else {
        return RepairReport {
            status: "corrupt".into(),
            db_path,
            messages,
            post_repair_messages: Vec::new(),
            backup_path,
            reindexed: Vec::new(),
        };
    };
    let mut reindexed = Vec::new();
    for name in &index_names {
        let targeted = conn.execute_batch(&format!("REINDEX \"{name}\";"));
        if targeted.is_err() {
            // Parsed name did not resolve — fall back to a full REINDEX.
            if conn.execute_batch("REINDEX;").is_err() {
                return RepairReport {
                    status: "corrupt".into(),
                    db_path,
                    messages,
                    post_repair_messages: vec!["REINDEX failed".into()],
                    backup_path,
                    reindexed,
                };
            }
            reindexed = index_names.clone();
            break;
        }
        reindexed.push(name.clone());
    }
    let post = integrity_check(&conn);
    if integrity_ok(&post) {
        RepairReport {
            status: "repaired".into(),
            db_path,
            messages,
            post_repair_messages: post,
            backup_path,
            reindexed,
        }
    } else {
        RepairReport {
            status: "corrupt".into(),
            db_path,
            messages,
            post_repair_messages: post,
            backup_path,
            reindexed,
        }
    }
}

/// In-process slash surface for the REPL `/kanban` command (hermes
/// `kanban.run_slash`): one formatted string back, no process spawn.
pub fn run_slash(home: &std::path::Path, rest: &str) -> String {
    let mut parts = rest.split_whitespace();
    let sub = parts.next().unwrap_or("list");
    let store = match KanbanStore::open(home.join("kanban.db")) {
        Ok(s) => s,
        Err(e) => return format!("(._.) kanban error: {e}\n"),
    };
    match sub {
        "boards" => {
            let mut out = String::new();
            let current = store.current_board().unwrap_or_default();
            match store.list_boards() {
                Ok(boards) => {
                    for b in boards {
                        let marker = if b.slug == current { "*" } else { " " };
                        out.push_str(&format!("{marker} {:<20} {}\n", b.slug, b.name));
                    }
                }
                Err(e) => return format!("(._.) kanban error: {e}\n"),
            }
            out
        }
        "list" => {
            let status = parts.next();
            match store.list_tasks(None, status, None, 50) {
                Ok(tasks) => {
                    if tasks.is_empty() {
                        return "(o_o) no tasks on this board\n".to_string();
                    }
                    let mut out = String::new();
                    for t in tasks {
                        let assignee = t.assignee.as_deref().unwrap_or("(unassigned)");
                        out.push_str(&format!(
                            "{} {}  {:<9} {:<20} {}\n",
                            status_icon(&t.status),
                            t.id,
                            t.status,
                            assignee,
                            t.title
                        ));
                    }
                    out
                }
                Err(e) => format!("(._.) kanban error: {e}\n"),
            }
        }
        "show" => {
            let Some(id) = parts.next() else {
                return "(o_o) usage: /kanban show <id>\n".to_string();
            };
            let id = match resolve_any_id(&store, id) {
                Ok(Some(id)) => id,
                Ok(None) => return format!("(._.) task '{id}' not found\n"),
                Err(e) => return format!("(._.) kanban error: {e}\n"),
            };
            let Some(task) = store.get_task(&id).ok().flatten() else {
                return format!("(._.) task '{id}' not found\n")
            };
            let mut out = format!(
                "{} {}  [{}]  {}\n",
                status_icon(&task.status),
                task.id,
                task.status,
                task.title
            );
            if !task.body.is_empty() {
                out.push_str(&format!("{}\n", task.body));
            }
            if let Ok(comments) = store.comments(&id) {
                for c in comments {
                    out.push_str(&format!("  — {}: {}\n", c.author, c.body));
                }
            }
            out
        }
        "create" => {
            let title: Vec<&str> = parts.collect();
            if title.is_empty() {
                return "(o_o) usage: /kanban create <title>\n".to_string();
            }
            match store.create_task(&NewTask {
                title: title.join(" "),
                created_by: "repl".into(),
                ..Default::default()
            }) {
                Ok(t) => format!("{} {} (todo)  {}\n", status_icon("todo"), t.id, t.title),
                Err(e) => format!("(._.) kanban error: {e}\n"),
            }
        }
        "done" | "complete" => {
            let Some(id) = parts.next() else {
                return "(o_o) usage: /kanban done <id> [result]\n".to_string();
            };
            let result: Vec<&str> = parts.collect();
            let id = match resolve_any_id(&store, id) {
                Ok(Some(id)) => id,
                _ => return format!("(._.) task '{id}' not found\n"),
            };
            let joined = result.join(" ");
            match store.complete_task(&id, if joined.is_empty() { None } else { Some(&joined) }) {
                Ok(t) => format!("✓ {} done\n", t.id),
                Err(e) => format!("(._.) kanban error: {e}\n"),
            }
        }
        "block" => {
            let Some(id) = parts.next() else {
                return "(o_o) usage: /kanban block <id> <reason>\n".to_string();
            };
            let reason: Vec<&str> = parts.collect();
            if reason.is_empty() {
                return "(o_o) usage: /kanban block <id> <reason>\n".to_string();
            }
            let id = match resolve_any_id(&store, id) {
                Ok(Some(id)) => id,
                _ => return format!("(._.) task '{id}' not found\n"),
            };
            match store.block_task(&id, &reason.join(" ")) {
                Ok(t) => format!("⊘ {} blocked\n", t.id),
                Err(e) => format!("(._.) kanban error: {e}\n"),
            }
        }
        "unblock" => {
            let Some(id) = parts.next() else {
                return "(o_o) usage: /kanban unblock <id>\n".to_string();
            };
            let id = match resolve_any_id(&store, id) {
                Ok(Some(id)) => id,
                _ => return format!("(._.) task '{id}' not found\n"),
            };
            match store.unblock_task(&id) {
                Ok(t) => format!("▶ {} ready\n", t.id),
                Err(e) => format!("(._.) kanban error: {e}\n"),
            }
        }
        "comment" => {
            let Some(id) = parts.next() else {
                return "(o_o) usage: /kanban comment <id> <text>\n".to_string();
            };
            let text: Vec<&str> = parts.collect();
            if text.is_empty() {
                return "(o_o) usage: /kanban comment <id> <text>\n".to_string();
            }
            let id = match resolve_any_id(&store, id) {
                Ok(Some(id)) => id,
                _ => return format!("(._.) task '{id}' not found\n"),
            };
            match store.add_comment(&id, "repl", &text.join(" ")) {
                Ok(()) => format!("comment added to {}\n", id),
                Err(e) => format!("(._.) kanban error: {e}\n"),
            }
        }
        _ => {
            "(o_o) usage: /kanban [boards|list [status]|show <id>|create <title>|done <id> \
             [result]|block <id> <reason>|unblock <id>|comment <id> <text>]\n"
                .to_string()
        }
    }
}

/// Resolve an exact id or unique prefix against the store.
fn resolve_any_id(store: &KanbanStore, id: &str) -> Result<Option<String>> {
    if store.get_task(id)?.is_some() {
        return Ok(Some(id.to_string()));
    }
    store.resolve_task_id(id)
}

/// The task this process works on when spawned as a kanban worker
/// (hermes `HERMES_KANBAN_TASK`; ulnclaw native name first).
pub fn worker_task_env() -> Option<String> {
    crate::config::get_env_value("ULNCLAW_KANBAN_TASK")
        .or_else(|| crate::config::get_env_value("HERMES_KANBAN_TASK"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Whether the kanban stop-guard is active for this process (hermes
/// `kanban_stop_nudge_enabled`): on when a worker task is set, unless
/// `ULNCLAW_KANBAN_STOP_NUDGE` explicitly disables it.
pub fn stop_nudge_enabled() -> bool {
    if let Some(flag) = crate::config::get_env_value("ULNCLAW_KANBAN_STOP_NUDGE") {
        if matches!(flag.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
    }
    worker_task_env().is_some()
}

/// Default nudge budget (hermes `_DEFAULT_MAX_ATTEMPTS`).
pub const STOP_NUDGE_MAX_ATTEMPTS: usize = 2;

/// Synthetic follow-up for a worker that tried to finish without a
/// terminal board tool (hermes `build_kanban_stop_nudge`).
pub fn build_kanban_stop_nudge(task_id: &str, attempts: usize) -> Option<String> {
    if !stop_nudge_enabled() || attempts >= STOP_NUDGE_MAX_ATTEMPTS {
        return None;
    }
    let tid = if task_id.trim().is_empty() { "this task".to_string() } else { task_id.trim().to_string() };
    Some(format!(
        "[System: You are an ulnclaw kanban worker. A plain-text reply is NOT a \
         terminal state for the board.\n\n\
         Task `{tid}` is still `running`. Ending now without a board tool causes a \
         protocol violation (clean exit with no `kanban_complete` / `kanban_block`).\n\n\
         Do this immediately in your next response — do not narrate intent:\n\
         1. Finish any remaining deliverable (write the required file(s) now).\n\
         2. Call `kanban_complete(result=...)` if the work is done, OR \
         `kanban_block(result=<reason>)` if you are blocked.\n\n\
         Never end a turn with only a promise of future action. Repeated protocol \
         violations will block this task and require manual intervention.]"
    ))
}

// ---------------------------------------------------------------------------
// Kanban Swarm v1 (hermes kanban_swarm.py): a durable, immediately
// dispatchable graph — planning root (done, shared blackboard) → parallel
// workers (ready) → verifier (waits for every worker) → synthesizer (waits
// for the verifier). The dispatcher + parent-aware recompute_ready drive it.
// ---------------------------------------------------------------------------

/// One child card of a triage decomposition (hermes decompose children
/// dicts). `parents` are indices into the same children list.
#[derive(Debug, Clone)]
pub struct DecomposeChild {
    pub title: String,
    pub body: String,
    pub assignee: Option<String>,
    pub parents: Vec<usize>,
}

/// One parallel worker card in a swarm (hermes `SwarmWorkerSpec`).
#[derive(Debug, Clone)]
pub struct SwarmWorkerSpec {
    pub assignee: String,
    pub title: String,
    pub body: String,
    pub priority: i64,
    /// Skills force-loaded into this worker (hermes spec.skills).
    pub skills: Vec<String>,
}

/// Ids produced by [`KanbanStore::create_swarm`] (hermes `SwarmCreated`).
#[derive(Debug, Clone, Serialize)]
pub struct SwarmCreated {
    pub root_id: String,
    pub worker_ids: Vec<String>,
    pub verifier_id: String,
    pub synthesizer_id: String,
}

/// Shared protocol context appended to every swarm card (hermes
/// `_swarm_context`).
fn swarm_context(root_id: &str, goal: &str) -> String {
    format!(
        "\n\n## Swarm protocol\n\
         - Swarm root / shared blackboard: `{root_id}`.\n\
         - Read sibling/parent handoffs from Kanban context before working.\n\
         - Put machine-readable facts in completion metadata.\n\
         - Put cross-worker notes on the root task using structured comments.\n\
         - Goal: {goal}\n"
    )
}

impl KanbanStore {
    /// Create a durable swarm graph (hermes `create_swarm`). The root is
    /// completed immediately (it stays the shared blackboard + audit
    /// anchor), workers start `ready`, the verifier is linked to every
    /// worker, and the synthesizer to the verifier — so the dispatcher
    /// runs the whole pipeline without further orchestration.
    pub fn create_swarm(
        &self,
        goal: &str,
        workers: &[SwarmWorkerSpec],
        verifier_assignee: &str,
        synthesizer_assignee: &str,
        created_by: &str,
        idempotency_key: Option<&str>,
    ) -> Result<SwarmCreated> {
        let goal = goal.trim();
        if goal.is_empty() {
            return Err(AgentError::session("kanban swarm: goal is required"));
        }
        if workers.is_empty() {
            return Err(AgentError::session("kanban swarm: at least one worker is required"));
        }
        for (i, spec) in workers.iter().enumerate() {
            if spec.assignee.trim().is_empty() || spec.title.trim().is_empty() {
                return Err(AgentError::session(format!(
                    "kanban swarm: workers[{}].assignee and .title are required",
                    i + 1
                )));
            }
        }
        if verifier_assignee.trim().is_empty() {
            return Err(AgentError::session("kanban swarm: verifier assignee is required"));
        }
        if synthesizer_assignee.trim().is_empty() {
            return Err(AgentError::session("kanban swarm: synthesizer assignee is required"));
        }
        let created_by = if created_by.trim().is_empty() { "swarm-orchestrator" } else { created_by };

        let first_line = goal.lines().next().unwrap_or(goal);
        let root_title = format!("Swarm: {}", first_line.chars().take(80).collect::<String>());
        let root = self.create_task(&NewTask {
            title: root_title,
            body: format!(
                "Kanban Swarm v1 planning/root card. This card is completed \
                 immediately so parallel workers can start while it remains the \
                 shared blackboard and audit anchor.\n\nGoal:\n{goal}"
            ),
            assignee: Some(created_by.to_string()),
            priority: 0,
            tenant: None,
            model: None,
            created_by: created_by.to_string(),
            idempotency_key: idempotency_key.map(str::to_string),
            ..Default::default()
        })?;

        // Idempotent recovery (hermes create_swarm): when the key returned
        // an existing root, rebuild the topology from its blackboard
        // instead of duplicating the graph.
        if let Some(blackboard) = self.latest_blackboard(&root.id)? {
            if let Some(topology) = blackboard.get("topology") {
                let worker_ids: Vec<String> = topology
                    .get("worker_ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let verifier_id = topology
                    .get("verifier_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let synthesizer_id = topology
                    .get("synthesizer_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !worker_ids.is_empty() && !verifier_id.is_empty() && !synthesizer_id.is_empty() {
                    return Ok(SwarmCreated {
                        root_id: root.id,
                        worker_ids,
                        verifier_id,
                        synthesizer_id,
                    });
                }
            }
        }
        let context = swarm_context(&root.id, goal);

        let mut worker_ids = Vec::new();
        for spec in workers {
            let worker = self.create_task(&NewTask {
                title: spec.title.trim().to_string(),
                body: format!("{}{}", spec.body, context),
                assignee: Some(spec.assignee.trim().to_string()),
                priority: spec.priority,
                tenant: None,
                model: None,
                created_by: created_by.to_string(),
                skills: if spec.skills.is_empty() {
                    None
                } else {
                    Some(spec.skills.clone())
                },
                ..Default::default()
            })?;
            self.link_tasks(&root.id, &worker.id)?;
            self.ready_task(&worker.id)?;
            worker_ids.push(worker.id);
        }

        let verifier = self.create_task(&NewTask {
            title: "Verify swarm outputs".to_string(),
            body: format!(
                "Verify the completed worker tasks of this swarm against the goal.{}",
                context
            ),
            assignee: Some(verifier_assignee.trim().to_string()),
            priority: 0,
            tenant: None,
            model: None,
            created_by: created_by.to_string(),
            skills: Some(vec!["requesting-code-review".to_string()]),
            ..Default::default()
        })?;
        for worker_id in &worker_ids {
            self.link_tasks(worker_id, &verifier.id)?;
        }

        let synthesizer = self.create_task(&NewTask {
            title: "Synthesize swarm outputs".to_string(),
            body: format!(
                "Synthesize the verified swarm outputs into the final deliverable.{}",
                context
            ),
            assignee: Some(synthesizer_assignee.trim().to_string()),
            priority: 0,
            tenant: None,
            model: None,
            created_by: created_by.to_string(),
            skills: Some(vec!["humanizer".to_string()]),
            ..Default::default()
        })?;
        self.link_tasks(&verifier.id, &synthesizer.id)?;

        // Blackboard anchor: topology comment on the root + swarm event.
        let topology = serde_json::json!({
            "topology": {
                "root_id": root.id,
                "worker_ids": worker_ids,
                "verifier_id": verifier.id,
                "synthesizer_id": synthesizer.id,
            }
        });
        self.add_comment(&root.id, "blackboard", &topology.to_string())?;
        self.append_event(&root.id, "swarm", topology)?;
        self.complete_task(&root.id, Some("planning complete; topology on blackboard"))?;

        Ok(SwarmCreated {
            root_id: root.id,
            worker_ids,
            verifier_id: verifier.id,
            synthesizer_id: synthesizer.id,
        })
    }

    /// Fan a triage task out into a graph of children and promote the root
    /// to `todo` (hermes `decompose_triage_task`). The root stays alive as
    /// a CHILD of every decomposed child, so it wakes back up (todo→ready
    /// via `recompute_ready`) once the whole graph completes — its
    /// assignee (the orchestrator profile) then judges completion.
    /// Returns `Ok(None)` when the root vanished or moved out of triage.
    pub fn decompose_triage_task(
        &self,
        task_id: &str,
        root_assignee: Option<&str>,
        children: &[DecomposeChild],
        author: &str,
        auto_promote: bool,
    ) -> Result<Option<Vec<String>>> {
        if children.is_empty() {
            return Ok(None);
        }
        // Validate the sibling graph up front (hermes raises ValueError).
        for (idx, child) in children.iter().enumerate() {
            if child.title.trim().is_empty() {
                return Err(AgentError::session(format!(
                    "kanban decompose: child[{idx}].title is required"
                )));
            }
            for &parent_idx in &child.parents {
                if parent_idx >= children.len() {
                    return Err(AgentError::session(format!(
                        "kanban decompose: child[{idx}].parents index {parent_idx} out of range"
                    )));
                }
                if parent_idx == idx {
                    return Err(AgentError::session(format!(
                        "kanban decompose: child[{idx}] cannot list itself as a parent"
                    )));
                }
            }
        }
        // Kahn topological sort → cycle detection (a cycle would deadlock
        // every involved child in todo forever).
        let mut in_degree = vec![0usize; children.len()];
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); children.len()];
        for (idx, child) in children.iter().enumerate() {
            for &parent_idx in &child.parents {
                adjacency[parent_idx].push(idx);
                in_degree[idx] += 1;
            }
        }
        let mut queue: Vec<usize> = (0..children.len()).filter(|&i| in_degree[i] == 0).collect();
        let mut seen = 0usize;
        while let Some(node) = queue.pop() {
            seen += 1;
            for &next in &adjacency[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push(next);
                }
            }
        }
        if seen != children.len() {
            return Err(AgentError::session(
                "kanban decompose: cyclic dependency in children graph",
            ));
        }
        // Root must still be in triage before we create anything.
        let root_is_triage = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT 1 FROM tasks WHERE id = ?1 AND status = 'triage'",
                params![task_id],
                |_| Ok(()),
            )
            .is_ok()
        };
        if !root_is_triage {
            return Ok(None);
        }
        let author = if author.trim().is_empty() { "decomposer" } else { author.trim() };

        let mut child_ids = Vec::new();
        for child in children {
            let created = self.create_task(&NewTask {
                title: child.title.trim().chars().take(200).collect(),
                body: child.body.clone(),
                assignee: child.assignee.clone(),
                priority: 0,
                tenant: None,
                model: None,
                created_by: author.to_string(),
                ..Default::default()
            })?;
            self.append_event(
                &created.id,
                "decomposed",
                serde_json::json!({ "root": task_id, "by": author }),
            )?;
            child_ids.push(created.id);
        }
        // Sibling parent links (within the decomposed graph).
        for (idx, child) in children.iter().enumerate() {
            for &parent_idx in &child.parents {
                self.link_tasks(&child_ids[parent_idx], &child_ids[idx])?;
            }
        }
        // The root waits for the whole graph: root becomes a child of
        // every decomposed child (cycle-free — root is only ever a child).
        for child_id in &child_ids {
            self.link_tasks(child_id, task_id)?;
        }
        // Flip the root: triage → todo (+ orchestrator assignee), guarded.
        let updated = {
            let conn = self.conn.lock().unwrap();
            match root_assignee.map(str::trim).filter(|a| !a.is_empty()) {
                Some(assignee) => conn.execute(
                    "UPDATE tasks SET status = 'todo', assignee = ?2 \
                     WHERE id = ?1 AND status = 'triage'",
                    params![task_id, assignee],
                ),
                None => conn.execute(
                    "UPDATE tasks SET status = 'todo' WHERE id = ?1 AND status = 'triage'",
                    params![task_id],
                ),
            }
        }
        .map_err(db_error("decompose root flip"))?;
        if updated != 1 {
            // Race: root moved out of triage while we worked. Roll the
            // orphan children back so no dangling graph is left behind.
            for child_id in &child_ids {
                self.archive_task(child_id).ok();
            }
            return Ok(None);
        }
        self.add_comment(
            task_id,
            author,
            &format!(
                "Decomposed into {}. Root will wake when all children complete.",
                child_ids.join(", ")
            ),
        )?;
        self.append_event(
            task_id,
            "decomposed",
            serde_json::json!({ "child_ids": child_ids, "root_assignee": root_assignee }),
        )?;
        if auto_promote {
            self.recompute_ready()?;
        }
        Ok(Some(child_ids))
    }

    /// Post a cross-worker note to the swarm blackboard (structured comment
    /// on the root task; hermes `post_blackboard_update`).
    pub fn post_blackboard_update(&self, root_id: &str, author: &str, note: &str) -> Result<()> {
        self.add_comment(root_id, &format!("blackboard:{author}"), note)
    }

    /// Latest blackboard topology of a swarm root, if present (hermes
    /// `latest_blackboard`).
    pub fn latest_blackboard(&self, root_id: &str) -> Result<Option<Value>> {
        for comment in self.comments(root_id)?.into_iter().rev() {
            if !comment.author.starts_with("blackboard") {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&comment.body) {
                if value.get("topology").is_some() {
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Worker worktree isolation (hermes spawns each kanban worker in its own
// git worktree under <repo>/.worktrees/t_<hex> so parallel workers never
// share a dirty checkout)
// ---------------------------------------------------------------------------

/// The git repository enclosing `dir`, if any.
fn git_toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(path))
    }
}

/// Parse `--workspace` into `(kind, path|None)` (hermes
/// `_parse_workspace_flag`). Accepts `scratch`, `worktree`,
/// `worktree:<path>`, `dir:<path>`.
pub fn parse_workspace_flag(value: &str) -> std::result::Result<(String, Option<String>), String> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(("scratch".to_string(), None));
    }
    if v == "scratch" || v == "worktree" {
        return Ok((v.to_string(), None));
    }
    for (prefix, kind) in [("dir:", "dir"), ("worktree:", "worktree")] {
        if let Some(rest) = v.strip_prefix(prefix) {
            let path = rest.trim();
            if path.is_empty() {
                return Err(format!(
                    "--workspace {prefix} requires a path after the colon"
                ));
            }
            return Ok((
                kind.to_string(),
                Some(expand_tilde(path).to_string_lossy().to_string()),
            ));
        }
    }
    Err(format!(
        "unknown --workspace value {value:?}: use scratch, worktree, \
         worktree:<path>, or dir:<path>"
    ))
}

/// Validate `kanban create --branch` (hermes `_parse_branch_flag`).
pub fn parse_branch_flag(value: &str) -> std::result::Result<String, String> {
    let branch = value.trim();
    if branch.is_empty() {
        return Err("--branch requires a non-empty name".to_string());
    }
    if branch.starts_with('-') {
        return Err("--branch must not start with '-'".to_string());
    }
    if branch.chars().any(char::is_whitespace) {
        return Err("--branch must not contain whitespace".to_string());
    }
    Ok(branch.to_string())
}

/// Expand a leading `~/` against $HOME (hermes `os.path.expanduser`).
fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(&raw[2..]);
        }
    }
    PathBuf::from(raw)
}

/// Root of per-task scratch workspaces (hermes `workspaces_root`):
/// `<home>/kanban/workspaces/<task-id>`. Path-stable across retries so
/// handoff between workers reuses the same directory.
pub fn workspaces_root(home: &Path) -> PathBuf {
    home.join("kanban").join("workspaces")
}

/// Current branch of the repo checked out at `dir` (hermes
/// `_git_current_branch`).
fn git_current_branch(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() || name == "HEAD" {
        None
    } else {
        Some(name)
    }
}

/// True when `dir` is a LINKED git worktree checkout (`.git` is a file
/// pointing at the main repo, hermes `_is_linked_worktree_checkout`).
fn is_linked_worktree_checkout(dir: &Path) -> bool {
    dir.join(".git").is_file()
}

/// Create (or reuse) a linked worktree `target` in `repo` on `branch`
/// (hermes `_ensure_git_worktree`). Falls back to attaching an
/// existing branch when `-b` loses the race.
fn ensure_git_worktree(repo: &Path, target: &Path, branch: &str) -> std::result::Result<(), String> {
    if target.is_dir() {
        return Ok(());
    }
    let status = std::process::Command::new("git")
        .args(["worktree", "add", target.to_str().unwrap_or_default(), "-b", branch])
        .current_dir(repo)
        .status()
        .map_err(|e| format!("git worktree add: {e}"))?;
    if status.success() {
        return Ok(());
    }
    let status = std::process::Command::new("git")
        .args(["worktree", "add", target.to_str().unwrap_or_default(), branch])
        .current_dir(repo)
        .status()
        .map_err(|e| format!("git worktree add: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git worktree add failed for {}", target.display()))
    }
}

/// Prepare an isolated git worktree for `task` under
/// `<repo>/.worktrees/<task-id>` on branch `kanban/<task-id>` (hermes
/// worktree workspaces). Returns the worktree path, or `None` when the
/// dispatch cwd is not inside a git repo (the worker then runs in-place).
/// Existing worktrees are reused.
pub fn prepare_worktree(cwd: &Path, task: &Task) -> std::result::Result<Option<std::path::PathBuf>, String> {
    let Some(repo) = git_toplevel(cwd) else {
        return Ok(None);
    };
    let dir = repo.join(".worktrees").join(&task.id);
    if dir.is_dir() {
        return Ok(Some(dir));
    }
    let branch = format!("kanban/{}", task.id);
    let status = std::process::Command::new("git")
        .args(["worktree", "add", dir.to_str().unwrap_or_default(), "-b", &branch])
        .current_dir(&repo)
        .status()
        .map_err(|e| format!("git worktree add: {e}"))?;
    if !status.success() {
        // Branch may already exist from a previous run — attach to it.
        let status = std::process::Command::new("git")
            .args(["worktree", "add", dir.to_str().unwrap_or_default(), &branch])
            .current_dir(&repo)
            .status()
            .map_err(|e| format!("git worktree add: {e}"))?;
        if !status.success() {
            return Err(format!("git worktree add failed for {}", task.id));
        }
    }
    Ok(Some(dir))
}

/// Remove worktrees of tasks that reached a terminal status (hermes
/// dispatcher gc owns the `t_<hex>` trees). Branches are kept — that is
/// where the finished work lives. Returns (removed, skipped) counts.
pub fn gc_worktrees(cwd: &Path, store: &KanbanStore) -> std::result::Result<(usize, usize), String> {
    let Some(repo) = git_toplevel(cwd) else {
        return Ok((0, 0));
    };
    let root = repo.join(".worktrees");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok((0, 0));
    };
    let mut removed = 0usize;
    let mut skipped = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("t_") {
            continue;
        }
        let terminal = match store.get_task(&name) {
            Ok(Some(task)) => matches!(task.status.as_str(), "done" | "archived"),
            // Orphan tree (task row gone) — hermes gc reclaims these too.
            Ok(None) => true,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !terminal {
            skipped += 1;
            continue;
        }
        let status = std::process::Command::new("git")
            .args(["worktree", "remove", "--force", entry.path().to_str().unwrap_or_default()])
            .current_dir(&repo)
            .status();
        match status {
            Ok(s) if s.success() => removed += 1,
            _ => skipped += 1,
        }
    }
    Ok((removed, skipped))
}

/// Best-effort liveness check for a local pid (hermes `_pid_alive`).
fn pid_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Arguments for [`KanbanStore::create_task`].
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    pub body: String,
    pub assignee: Option<String>,
    pub priority: i64,
    pub tenant: Option<String>,
    pub model: Option<String>,
    pub created_by: String,
    /// Skills force-loaded into the dispatcher worker prompt.
    pub skills: Option<Vec<String>>,
    /// Per-attempt runtime cap (seconds) enforced by the dispatcher.
    pub max_runtime_seconds: Option<i64>,
    /// Dedup key: creating with the key of an existing non-archived task
    /// returns that task instead of a duplicate (hermes idempotency_key).
    pub idempotency_key: Option<String>,
    /// Park the task in the triage column (hermes `kanban create --triage`):
    /// a specifier/decomposer fleshes it out and promotes it to `todo`.
    pub triage: bool,
    /// Circuit-breaker threshold: block on the Nth failed attempt
    /// (hermes `max_retries`; 1 trips on the first failure).
    pub max_retries: Option<i64>,
    /// Workspace kind: `scratch` (default) | `worktree` | `dir`
    /// (hermes `create --workspace`).
    pub workspace_kind: Option<String>,
    /// Workspace path for `worktree` / `dir` kinds (hermes
    /// `worktree:<path>` / `dir:<path>`); must be absolute. When unset
    /// for those kinds the board `default_workdir` fills it in.
    pub workspace_path: Option<String>,
    /// Worktree branch name (hermes `create --branch`; only valid
    /// with the worktree kind).
    pub branch_name: Option<String>,
}

pub struct KanbanStore {
    pub(crate) conn: Mutex<Connection>,
    path: PathBuf,
}

fn db_error(context: &str) -> impl FnOnce(rusqlite::Error) -> AgentError + '_ {
    move |e| AgentError::session(format!("kanban {context}: {e}"))
}

impl KanbanStore {
    /// Open (or create) the board store at `path` and ensure schema +
    /// the default board exist (hermes `init_db` is idempotent too).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(&path).map_err(db_error("open"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(db_error("pragmas"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS boards (
                slug            TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                default_workdir TEXT,
                created_at      INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tasks (
                id              TEXT PRIMARY KEY,
                board           TEXT NOT NULL DEFAULT 'default',
                title           TEXT NOT NULL,
                body            TEXT NOT NULL DEFAULT '',
                assignee        TEXT,
                status          TEXT NOT NULL,
                priority        INTEGER NOT NULL DEFAULT 0,
                created_by      TEXT NOT NULL DEFAULT '',
                created_at      INTEGER NOT NULL,
                started_at      INTEGER,
                completed_at    INTEGER,
                tenant          TEXT,
                model           TEXT,
                result          TEXT,
                claim_lock      TEXT,
                claim_expires   INTEGER,
                last_heartbeat_at INTEGER,
                workspace_kind  TEXT NOT NULL DEFAULT 'scratch',
                workspace_path  TEXT,
                branch_name     TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_board_status ON tasks (board, status);
            CREATE TABLE IF NOT EXISTS task_comments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id     TEXT NOT NULL,
                author      TEXT NOT NULL,
                body        TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS task_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id     TEXT NOT NULL,
                kind        TEXT NOT NULL,
                payload     TEXT NOT NULL DEFAULT '{}',
                created_at  INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS task_links (
                parent_id   TEXT NOT NULL,
                child_id    TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                PRIMARY KEY (parent_id, child_id)
            );
            CREATE INDEX IF NOT EXISTS idx_links_child ON task_links (child_id);
            CREATE INDEX IF NOT EXISTS idx_links_parent ON task_links (parent_id);
            CREATE TABLE IF NOT EXISTS task_attachments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id     TEXT NOT NULL,
                kind        TEXT NOT NULL,
                value       TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS kanban_notify_subs (
                task_id           TEXT NOT NULL,
                platform          TEXT NOT NULL,
                chat_id           TEXT NOT NULL,
                chat_type         TEXT,
                thread_id         TEXT NOT NULL DEFAULT '',
                user_id           TEXT,
                notifier_profile  TEXT,
                delivery_metadata TEXT,
                created_at        INTEGER NOT NULL,
                last_event_id     INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (task_id, platform, chat_id, thread_id)
            );
            CREATE INDEX IF NOT EXISTS idx_notify_task ON kanban_notify_subs(task_id);
            CREATE TABLE IF NOT EXISTS task_runs (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id             TEXT NOT NULL,
                profile             TEXT,
                step_key            TEXT,
                status              TEXT NOT NULL,
                claim_lock          TEXT,
                claim_expires       INTEGER,
                worker_pid          INTEGER,
                max_runtime_seconds INTEGER,
                last_heartbeat_at   INTEGER,
                started_at          INTEGER NOT NULL,
                ended_at            INTEGER,
                outcome             TEXT,
                summary             TEXT,
                metadata            TEXT,
                error               TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_runs_task ON task_runs(task_id, started_at);
            CREATE INDEX IF NOT EXISTS idx_runs_status ON task_runs(status);",
        )
        .map_err(db_error("schema"))?;
        // Additive migrations: pre-P122 stores lack worker_pid; pre-P127
        // stores lack skills / max_runtime_seconds / idempotency_key
        // (hermes kanban_db column backfills).
        let columns: std::collections::HashSet<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(tasks)")
                .map_err(db_error("migrate"))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(db_error("migrate"))?;
            rows.flatten().collect()
        };
        if !columns.contains("worker_pid") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN worker_pid INTEGER;")
                .map_err(db_error("migrate worker_pid"))?;
        }
        if !columns.contains("skills") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN skills TEXT;")
                .map_err(db_error("migrate skills"))?;
        }
        if !columns.contains("max_runtime_seconds") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN max_runtime_seconds INTEGER;")
                .map_err(db_error("migrate max_runtime_seconds"))?;
        }
        if !columns.contains("idempotency_key") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN idempotency_key TEXT;")
                .map_err(db_error("migrate idempotency_key"))?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_idempotency ON tasks(idempotency_key);",
        )
        .map_err(db_error("migrate idempotency index"))?;
        // Pre-P132 stores lack the active-run pointer (hermes
        // tasks.current_run_id).
        if !columns.contains("current_run_id") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN current_run_id INTEGER;")
                .map_err(db_error("migrate current_run_id"))?;
        }
        // Pre-P136 stores lack the unified failure-accounting columns
        // (hermes consecutive_failures / last_failure_error / max_retries).
        if !columns.contains("consecutive_failures") {
            conn.execute_batch(
                "ALTER TABLE tasks ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;",
            )
            .map_err(db_error("migrate consecutive_failures"))?;
        }
        if !columns.contains("last_failure_error") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN last_failure_error TEXT;")
                .map_err(db_error("migrate last_failure_error"))?;
        }
        if !columns.contains("max_retries") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN max_retries INTEGER;")
                .map_err(db_error("migrate max_retries"))?;
        }
        // Pre-P139 stores lack the workspace columns (hermes
        // workspace_kind / workspace_path / branch_name).
        if !columns.contains("workspace_kind") {
            conn.execute_batch(
                "ALTER TABLE tasks ADD COLUMN workspace_kind TEXT NOT NULL DEFAULT 'scratch';",
            )
            .map_err(db_error("migrate workspace_kind"))?;
        }
        if !columns.contains("workspace_path") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN workspace_path TEXT;")
                .map_err(db_error("migrate workspace_path"))?;
        }
        if !columns.contains("branch_name") {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN branch_name TEXT;")
                .map_err(db_error("migrate branch_name"))?;
        }
        let store = Self {
            conn: Mutex::new(conn),
            path,
        };
        let now = Self::now();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT OR IGNORE INTO boards (slug, name, created_at) VALUES (?1, ?1, ?2)",
                params![DEFAULT_BOARD, now],
            )
            .map_err(db_error("seed board"))?;
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES ('current_board', ?1)",
                params![DEFAULT_BOARD],
            )
            .map_err(db_error("seed meta"))?;
        Ok(store)
    }

    /// Open the default store at `<home>/kanban.db`.
    pub fn open_default() -> Result<Self> {
        Self::open(crate::config::ulnclaw_home().join("kanban.db"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Hermes `_new_task_id`: `t_` + 4 random hex bytes.
    pub fn new_task_id() -> String {
        let raw = uuid::Uuid::new_v4();
        let hex: String = raw
            .as_bytes()
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("t_{hex}")
    }

    /// Hermes `_claimer_id`: `host:pid`.
    pub fn claimer_id() -> String {
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "localhost".to_string());
        format!("{}:{}", host, std::process::id())
    }

    // --------------------------------------------------------------- boards

    pub fn current_board(&self) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let board: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'current_board'",
                [],
                |row| row.get(0),
            )
            .map_err(db_error("current board"))?;
        Ok(board)
    }

    pub fn list_boards(&self) -> Result<Vec<Board>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT slug, name, default_workdir, created_at FROM boards ORDER BY created_at")
            .map_err(db_error("boards prepare"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Board {
                    slug: row.get(0)?,
                    name: row.get(1)?,
                    default_workdir: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .map_err(db_error("boards query"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("boards collect"))
    }

    pub fn create_board(
        &self,
        slug: &str,
        name: Option<&str>,
        default_workdir: Option<&str>,
    ) -> Result<()> {
        let slug = slug.trim().to_lowercase();
        if slug.is_empty() || slug.contains(char::is_whitespace) {
            return Err(AgentError::session(
                "kanban: board slug must be non-empty without spaces",
            ));
        }
        let conn = self.conn.lock().unwrap();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO boards (slug, name, default_workdir, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    slug,
                    name.unwrap_or(&slug),
                    default_workdir,
                    Self::now()
                ],
            )
            .map_err(db_error("create board"))?;
        if inserted == 0 {
            return Err(AgentError::session(format!(
                "kanban: board '{slug}' already exists"
            )));
        }
        Ok(())
    }

    pub fn switch_board(&self, slug: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM boards WHERE slug = ?1",
                params![slug],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error("switch board"))?
            > 0;
        if !exists {
            return Err(AgentError::session(format!(
                "kanban: board '{slug}' not found"
            )));
        }
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'current_board'",
            params![slug],
        )
        .map_err(db_error("switch board"))?;
        Ok(())
    }

    pub fn remove_board(&self, slug: &str) -> Result<()> {
        if slug == DEFAULT_BOARD {
            return Err(AgentError::session("kanban: cannot remove the default board"));
        }
        let conn = self.conn.lock().unwrap();
        let tasks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE board = ?1 AND status != 'archived'",
                params![slug],
                |row| row.get(0),
            )
            .map_err(db_error("remove board"))?;
        if tasks > 0 {
            return Err(AgentError::session(format!(
                "kanban: board '{slug}' still has {tasks} active task(s) — archive them first"
            )));
        }
        let deleted = conn
            .execute("DELETE FROM boards WHERE slug = ?1", params![slug])
            .map_err(db_error("remove board"))?;
        if deleted == 0 {
            return Err(AgentError::session(format!(
                "kanban: board '{slug}' not found"
            )));
        }
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'current_board' AND value = ?2",
            params![DEFAULT_BOARD, slug],
        )
        .map_err(db_error("remove board"))?;
        Ok(())
    }

    /// (slug, total tasks, active tasks) per board (hermes
    /// `_board_task_counts`).
    /// Rename a board's display name (hermes `boards rename`).
    pub fn rename_board(&self, slug: &str, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            return Err(AgentError::session("kanban: board name cannot be blank"));
        }
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE boards SET name = ?2 WHERE slug = ?1",
                params![slug, name.trim()],
            )
            .map_err(db_error("boards rename"))?;
        if updated == 0 {
            return Err(AgentError::session(format!("kanban: board {slug} not found")));
        }
        Ok(())
    }

    /// Set a board's default working directory (hermes `boards set-workdir`).
    pub fn set_board_workdir(&self, slug: &str, workdir: Option<&str>) -> Result<()> {
        let workdir = workdir.map(str::trim).filter(|w| !w.is_empty());
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE boards SET default_workdir = ?2 WHERE slug = ?1",
                params![slug, workdir],
            )
            .map_err(db_error("boards set-workdir"))?;
        if updated == 0 {
            return Err(AgentError::session(format!("kanban: board {slug} not found")));
        }
        Ok(())
    }

    /// Per-status + per-assignee counts plus oldest-ready age of the
    /// current board (hermes `board_stats`).
    pub fn board_stats(&self) -> Result<BoardStats> {
        let board = self.current_board()?;
        let now = Self::now();
        let (by_status, rows, oldest) = {
            let conn = self.conn.lock().unwrap();
            let by_status: Vec<(String, i64)> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT status, COUNT(*) FROM tasks WHERE board = ?1 \
                         AND status != 'archived' GROUP BY status",
                    )
                    .map_err(db_error("stats"))?;
                let found = stmt
                    .query_map(params![board], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })
                    .map_err(db_error("stats"))?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_error("stats"))?;
                found
            };
            let rows: Vec<(String, String, i64)> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT assignee, status, COUNT(*) FROM tasks WHERE board = ?1 \
                         AND status != 'archived' AND assignee IS NOT NULL \
                         GROUP BY assignee, status ORDER BY assignee",
                    )
                    .map_err(db_error("stats"))?;
                let found = stmt
                    .query_map(params![board], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })
                    .map_err(db_error("stats"))?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_error("stats"))?;
                found
            };
            let oldest: Option<i64> = conn
                .query_row(
                    "SELECT MIN(created_at) FROM tasks WHERE board = ?1 AND status = 'ready'",
                    params![board],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .unwrap_or(None);
            (by_status, rows, oldest)
        };
        let mut by_assignee: Vec<(String, Vec<(String, i64)>)> = Vec::new();
        for (assignee, status, count) in rows {
            match by_assignee.iter_mut().find(|(name, _)| *name == assignee) {
                Some((_, counts)) => counts.push((status, count)),
                None => by_assignee.push((assignee, vec![(status, count)])),
            }
        }
        Ok(BoardStats {
            by_status,
            by_assignee,
            oldest_ready_age_seconds: oldest.map(|ts| now - ts),
            now,
        })
    }

    /// Highest task_events row id (watch start point; 0 when empty).
    pub fn last_event_id(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM task_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0))
    }

    /// Board-wide event stream newer than `after_id` for `kanban watch`
    /// (hermes watch backend): optional assignee + kind filters, each hit
    /// paired with its task title.
    pub fn board_events_since(
        &self,
        after_id: i64,
        assignee: Option<&str>,
        kinds: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<(TaskEvent, String)>> {
        let board = self.current_board()?;
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT e.id, e.task_id, e.kind, e.payload, e.created_at, t.title \
             FROM task_events e JOIN tasks t ON t.id = e.task_id \
             WHERE t.board = ?1 AND e.id > ?2",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(board), Box::new(after_id)];
        if let Some(assignee) = assignee {
            sql.push_str(" AND t.assignee = ?3");
            param_values.push(Box::new(assignee.to_string()));
        }
        if let Some(kinds) = kinds.filter(|k| !k.is_empty()) {
            let placeholders: Vec<String> = (0..kinds.len())
                .map(|i| format!("?{}", param_values.len() + 1 + i))
                .collect();
            sql.push_str(&format!(" AND e.kind IN ({})", placeholders.join(", ")));
            for kind in kinds {
                param_values.push(Box::new(kind.clone()));
            }
        }
        sql.push_str(&format!(" ORDER BY e.id ASC LIMIT {}", limit.max(1)));
        let params: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(db_error("watch"))?;
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                let payload: String = row.get(3)?;
                Ok((
                    TaskEvent {
                        id: row.get(0)?,
                        task_id: row.get(1)?,
                        kind: row.get(2)?,
                        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                        created_at: row.get(4)?,
                    },
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(db_error("watch"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("watch"))
    }

    /// Per-status task counts of the current board (hermes `kanban stats`).
    pub fn board_status_counts(&self) -> Result<Vec<(String, i64)>> {
        let board = self.current_board()?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT status, COUNT(*) FROM tasks WHERE board = ?1 GROUP BY status")
            .map_err(db_error("stats"))?;
        let rows = stmt
            .query_map(params![board], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(db_error("stats"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("stats"))
    }

    pub fn board_task_counts(&self) -> Result<Vec<(String, i64, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT b.slug,
                        (SELECT COUNT(*) FROM tasks t WHERE t.board = b.slug),
                        (SELECT COUNT(*) FROM tasks t WHERE t.board = b.slug
                           AND t.status NOT IN ('done', 'archived'))
                 FROM boards b ORDER BY b.created_at",
            )
            .map_err(db_error("board counts"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(db_error("board counts"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("board counts"))
    }

    // ---------------------------------------------------------------- tasks

    pub fn create_task(&self, task: &NewTask) -> Result<Task> {
        if task.title.trim().is_empty() {
            return Err(AgentError::session("kanban: task title is required"));
        }
        // Idempotency (hermes create_task): an existing non-archived task
        // with the same key is returned as-is instead of duplicating.
        let idempotency_key = task
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
        if let Some(key) = &idempotency_key {
            let existing = {
                let conn = self.conn.lock().unwrap();
                conn.query_row(
                    "SELECT id FROM tasks WHERE idempotency_key = ?1 \
                     AND status != 'archived' ORDER BY created_at ASC LIMIT 1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            };
            if let Some(found_id) = existing {
                if let Some(found) = self.get_task(&found_id)? {
                    return Ok(found);
                }
            }
        }
        // Workspace validation (hermes create_task).
        let workspace_kind = task
            .workspace_kind
            .as_deref()
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .unwrap_or("scratch")
            .to_string();
        if !VALID_WORKSPACE_KINDS.contains(&workspace_kind.as_str()) {
            return Err(AgentError::session(format!(
                "kanban: workspace_kind must be one of scratch, worktree, dir \
                 (got '{workspace_kind}')"
            )));
        }
        let mut workspace_path = task
            .workspace_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string);
        let branch_name = task
            .branch_name
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(str::to_string);
        if branch_name.is_some() && workspace_kind != "worktree" {
            return Err(AgentError::session(
                "kanban: branch_name is only valid for worktree workspaces",
            ));
        }
        let id = Self::new_task_id();
        let board = self.current_board()?;
        // Resolve workspace_path from the board default_workdir when the
        // caller named a dir/worktree workspace without a path (hermes
        // create_task board-level fill-in).
        if workspace_path.is_none() && matches!(workspace_kind.as_str(), "dir" | "worktree") {
            let conn = self.conn.lock().unwrap();
            let default_workdir: Option<String> = conn
                .query_row(
                    "SELECT default_workdir FROM boards WHERE slug = ?1",
                    params![board],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            drop(conn);
            workspace_path = default_workdir
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string);
        }
        let now = Self::now();
        let initial_status = if task.triage { "triage" } else { "todo" };
        let skills_json = task
            .skills
            .as_ref()
            .filter(|skills| !skills.is_empty())
            .map(|skills| serde_json::to_string(skills).unwrap_or_default());
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (id, board, title, body, assignee, status, priority, \
             created_by, created_at, tenant, model, skills, max_runtime_seconds, \
             idempotency_key, max_retries, workspace_kind, workspace_path, branch_name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                     ?16, ?17, ?18)",
            params![
                id,
                board,
                task.title.trim(),
                task.body,
                task.assignee,
                initial_status,
                task.priority,
                task.created_by,
                now,
                task.tenant,
                task.model,
                skills_json,
                task.max_runtime_seconds,
                idempotency_key,
                task.max_retries,
                workspace_kind,
                workspace_path,
                branch_name,
            ],
        )
        .map_err(db_error("create task"))?;
        drop(conn);
        self.append_event(&id, "created", serde_json::json!({ "board": board }))?;
        self.get_task(&id)?
            .ok_or_else(|| AgentError::session("kanban: task vanished after create"))
    }

    /// Persist the resolved workspace path so subsequent runs reuse the
    /// same directory (hermes `set_workspace_path`).
    pub fn set_workspace_path(&self, task_id: &str, path: &Path) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET workspace_path = ?1 WHERE id = ?2",
            params![path.to_string_lossy().to_string(), task_id],
        )
        .map_err(db_error("set workspace_path"))?;
        Ok(())
    }

    /// Persist the resolved worktree branch (hermes `set_branch_name`).
    pub fn set_branch_name(&self, task_id: &str, branch: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET branch_name = ?1 WHERE id = ?2",
            params![branch, task_id],
        )
        .map_err(db_error("set branch_name"))?;
        Ok(())
    }

    /// Resolve (and create if needed) the workspace for `task` (hermes
    /// `resolve_workspace`). Returns the workspace directory plus the
    /// resolved branch name for worktree workspaces.
    ///
    /// - `scratch`: a fresh dir under `<home>/kanban/workspaces/<id>`.
    ///   A legacy explicit `workspace_path` must be absolute.
    /// - `dir`: the stored `workspace_path` (must be absolute —
    ///   relative paths are rejected to prevent confused-deputy
    ///   traversal against the dispatcher's CWD). Created if missing.
    /// - `worktree`: a linked git worktree; see
    ///   [`Self::resolve_worktree_workspace`].
    pub fn resolve_workspace(
        &self,
        home: &Path,
        task: &Task,
    ) -> std::result::Result<(PathBuf, Option<String>), String> {
        let kind = if task.workspace_kind.trim().is_empty() {
            "scratch"
        } else {
            task.workspace_kind.as_str()
        };
        let explicit = task
            .workspace_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        match kind {
            "scratch" => {
                let path = match explicit {
                    Some(raw) => {
                        let p = expand_tilde(raw);
                        if !p.is_absolute() {
                            return Err(format!(
                                "task {} has non-absolute workspace_path {:?}; \
                                 workspace paths must be absolute",
                                task.id, raw
                            ));
                        }
                        p
                    }
                    None => workspaces_root(home).join(&task.id),
                };
                std::fs::create_dir_all(&path)
                    .map_err(|e| format!("create {}: {e}", path.display()))?;
                Ok((path, None))
            }
            "dir" => {
                let Some(raw) = explicit else {
                    return Err(format!(
                        "task {} has workspace_kind=dir but no workspace_path",
                        task.id
                    ));
                };
                let p = expand_tilde(raw);
                if !p.is_absolute() {
                    return Err(format!(
                        "task {} has non-absolute workspace_path {:?}; use an \
                         absolute path (relative paths are ambiguous against \
                         the dispatcher's CWD)",
                        task.id, raw
                    ));
                }
                std::fs::create_dir_all(&p)
                    .map_err(|e| format!("create {}: {e}", p.display()))?;
                Ok((p, None))
            }
            "worktree" => self.resolve_worktree_workspace(task),
            other => Err(format!("unknown workspace_kind: {other}")),
        }
    }

    /// Resolve + materialize a linked git worktree for `task` (hermes
    /// `_resolve_worktree_workspace`).
    ///
    /// Without `workspace_path` the anchor is the task board's
    /// `default_workdir`; boards without one fall back to the
    /// dispatcher's CWD (the pre-P139 ulnclaw behaviour) and a cwd
    /// outside any git repo fails loudly instead of guessing.
    ///
    /// With `workspace_path`: a repo root anchors a fresh
    /// `<repo>/.worktrees/<task-id>`; an existing linked checkout is
    /// reused only when it is already on the task's branch (otherwise a
    /// sibling task owns it and we materialize our own worktree under
    /// the same repo); any other path inside a repo becomes the
    /// worktree target itself. Empty `branch_name` defaults to
    /// `wt/<task-id>`.
    fn resolve_worktree_workspace(
        &self,
        task: &Task,
    ) -> std::result::Result<(PathBuf, Option<String>), String> {
        let branch_name = task
            .branch_name
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("wt/{}", task.id));
        let explicit = task
            .workspace_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let Some(raw) = explicit else {
            let default_workdir: Option<String> = {
                let conn = self.conn.lock().unwrap();
                conn.query_row(
                    "SELECT default_workdir FROM boards WHERE slug = ?1",
                    params![task.board],
                    |row| row.get(0),
                )
                .ok()
                .flatten()
            };
            let anchor = match default_workdir
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            {
                Some(raw) => {
                    let p = expand_tilde(raw);
                    if !p.is_absolute() {
                        return Err(format!(
                            "board {:?} default_workdir {:?} is not absolute; \
                             use an absolute path to a git repo",
                            task.board, raw
                        ));
                    }
                    p
                }
                // Legacy fallback (pre-P139 behaviour): anchor on the
                // dispatcher's CWD when the board has no workdir.
                None => std::env::current_dir().map_err(|e| format!("current dir: {e}"))?,
            };
            let repo_root = git_toplevel(&anchor).ok_or_else(|| {
                format!(
                    "task {} has workspace_kind=worktree but no workspace_path, \
                     and board {:?} has no usable default_workdir git repo. Set a \
                     board default workdir (kanban boards set-workdir) or create \
                     the task with --workspace worktree:<absolute-repo-path>",
                    task.id, task.board
                )
            })?;
            let target = repo_root.join(".worktrees").join(&task.id);
            ensure_git_worktree(&repo_root, &target, &branch_name)?;
            return Ok((target, Some(branch_name)));
        };

        let requested = expand_tilde(raw);
        if !requested.is_absolute() {
            return Err(format!(
                "task {} has non-absolute worktree path {:?}; use an absolute path",
                task.id, raw
            ));
        }
        if requested.exists() && is_linked_worktree_checkout(&requested) {
            let actual_branch = git_current_branch(&requested);
            if actual_branch.as_deref() == Some(branch_name.as_str()) {
                return Ok((requested, Some(branch_name)));
            }
            // The requested path is an existing checkout of a DIFFERENT
            // task's branch (decompose children inherit the root's
            // workspace_path verbatim). Reusing it would run this task
            // on the other task's branch — materialize our own
            // worktree under the same repo instead.
            if let Some(fallback_root) = requested.parent().and_then(git_toplevel) {
                let fallback = fallback_root.join(".worktrees").join(&task.id);
                if fallback != requested {
                    ensure_git_worktree(&fallback_root, &fallback, &branch_name)?;
                    return Ok((fallback, Some(branch_name)));
                }
            }
            return Ok((
                requested,
                Some(actual_branch.unwrap_or(branch_name)),
            ));
        }
        if let Some(repo_root) = git_toplevel(&requested) {
            if requested == repo_root {
                let target = repo_root.join(".worktrees").join(&task.id);
                ensure_git_worktree(&repo_root, &target, &branch_name)?;
                return Ok((target, Some(branch_name)));
            }
        }
        let repo_root = requested.parent().and_then(git_toplevel).ok_or_else(|| {
            format!(
                "task {} worktree path {:?} is not inside a git repo and does \
                 not point at a git repo root",
                task.id, raw
            )
        })?;
        ensure_git_worktree(&repo_root, &requested, &branch_name)?;
        Ok((requested, Some(branch_name)))
    }

    /// Flesh out a triage task and promote it to `todo` (hermes
    /// `specify_triage_task`). Atomically updates title/body/assignee
    /// (when provided) and flips triage→todo in one write. Returns false
    /// when the task is missing or not in the triage column — callers
    /// surface that as "nothing to specify", not an error. Landing on
    /// `todo` (not `ready`) keeps parent gating intact: `recompute_ready`
    /// promotes parent-free todos on the next dispatcher tick.
    pub fn specify_triage_task(
        &self,
        task_id: &str,
        title: Option<&str>,
        body: Option<&str>,
        assignee: Option<&str>,
        author: &str,
    ) -> Result<bool> {
        if let Some(candidate) = title {
            if candidate.trim().is_empty() {
                return Err(AgentError::session("kanban: title cannot be blank"));
            }
        }
        let conn = self.conn.lock().unwrap();
        let existing = conn
            .query_row(
                "SELECT title, body, assignee FROM tasks WHERE id = ?1 AND status = 'triage'",
                params![task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .ok();
        let Some((old_title, old_body, old_assignee)) = existing else {
            return Ok(false);
        };
        let new_title = title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(&old_title);
        let new_body = body.map_or_else(|| old_body.clone(), str::to_string);
        let new_assignee = assignee
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string)
            .or_else(|| old_assignee.clone());
        let changed = new_title != old_title
            || new_body != old_body
            || new_assignee.as_deref() != old_assignee.as_deref();
        let updated = conn
            .execute(
                "UPDATE tasks SET status = 'todo', title = ?2, body = ?3, assignee = ?4 \
                 WHERE id = ?1 AND status = 'triage'",
                params![task_id, new_title, new_body, new_assignee],
            )
            .map_err(db_error("specify"))?;
        if updated != 1 {
            return Ok(false);
        }
        if changed && !author.trim().is_empty() {
            conn.execute(
                "INSERT INTO task_comments (task_id, author, body, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    task_id,
                    author.trim(),
                    "Specified: task spec updated, promoted triage → todo.",
                    Self::now(),
                ],
            )
            .map_err(db_error("specify comment"))?;
        }
        drop(conn);
        self.append_event(task_id, "specified", serde_json::json!({}))?;
        Ok(true)
    }

    fn task_from_row(row: &rusqlite::Row) -> rusqlite::Result<Task> {
        Ok(Task {
            id: row.get("id")?,
            board: row.get("board")?,
            title: row.get("title")?,
            body: row.get("body")?,
            assignee: row.get("assignee")?,
            status: row.get("status")?,
            priority: row.get("priority")?,
            created_by: row.get("created_by")?,
            created_at: row.get("created_at")?,
            started_at: row.get("started_at")?,
            completed_at: row.get("completed_at")?,
            tenant: row.get("tenant")?,
            model: row.get("model")?,
            result: row.get("result")?,
            claim_lock: row.get("claim_lock")?,
            claim_expires: row.get("claim_expires")?,
            last_heartbeat_at: row.get("last_heartbeat_at")?,
            worker_pid: row.get("worker_pid")?,
            skills: row
                .get::<_, Option<String>>("skills")?
                .and_then(|raw| serde_json::from_str(&raw).ok()),
            max_runtime_seconds: row.get("max_runtime_seconds")?,
            idempotency_key: row.get("idempotency_key")?,
            consecutive_failures: row.get("consecutive_failures")?,
            last_failure_error: row.get("last_failure_error")?,
            max_retries: row.get("max_retries")?,
            workspace_kind: row.get("workspace_kind")?,
            workspace_path: row.get("workspace_path")?,
            branch_name: row.get("branch_name")?,
        })
    }

    const TASK_COLUMNS: &'static str = "id, board, title, body, assignee, status, priority, \
        created_by, created_at, started_at, completed_at, tenant, model, result, \
        claim_lock, claim_expires, last_heartbeat_at, worker_pid, skills, \
        max_runtime_seconds, idempotency_key, consecutive_failures, \
        last_failure_error, max_retries, workspace_kind, workspace_path, branch_name";

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!("SELECT {} FROM tasks WHERE id = ?1", Self::TASK_COLUMNS))
            .map_err(db_error("get task"))?;
        let mut rows = stmt
            .query_map(params![id], Self::task_from_row)
            .map_err(db_error("get task"))?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(db_error("get task"))?)),
            None => Ok(None),
        }
    }

    /// Resolve a full task id or unique prefix (hermes `resolve_task`).
    pub fn resolve_task_id(&self, prefix: &str) -> Result<Option<String>> {
        let prefix = prefix.trim();
        if prefix.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM tasks WHERE id = ?1 OR id LIKE ?2")
            .map_err(db_error("resolve task"))?;
        let rows = stmt
            .query_map(params![prefix, format!("{prefix}%")], |row| {
                row.get::<_, String>(0)
            })
            .map_err(db_error("resolve task"))?;
        let matches: Vec<String> = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("resolve task"))?;
        if matches.len() == 1 {
            Ok(Some(matches.into_iter().next().unwrap()))
        } else {
            // Ambiguous or unknown — callers report; exact matches win.
            Ok(matches.into_iter().find(|id| id == prefix))
        }
    }

    pub fn list_tasks(
        &self,
        board: Option<&str>,
        status: Option<&str>,
        assignee: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Task>> {
        let board = match board {
            Some(b) => b.to_string(),
            None => self.current_board()?,
        };
        let conn = self.conn.lock().unwrap();
        let mut sql = format!(
            "SELECT {} FROM tasks WHERE board = ?1",
            Self::TASK_COLUMNS
        );
        let mut bindings: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(board)];
        if let Some(status) = status {
            sql.push_str(" AND status = ?");
            bindings.push(Box::new(status.to_string()));
        }
        if let Some(assignee) = assignee {
            sql.push_str(" AND assignee = ?");
            bindings.push(Box::new(assignee.to_string()));
        }
        sql.push_str(" ORDER BY priority DESC, created_at DESC LIMIT ?");
        bindings.push(Box::new(limit as i64));
        let params: Vec<&dyn rusqlite::types::ToSql> =
            bindings.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(db_error("list tasks"))?;
        let rows = stmt
            .query_map(params.as_slice(), Self::task_from_row)
            .map_err(db_error("list tasks"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("list tasks"))
    }

    pub(crate) fn append_event(&self, task_id: &str, kind: &str, payload: Value) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO task_events (task_id, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, kind, payload.to_string(), Self::now()],
        )
        .map_err(db_error("append event"))?;
        Ok(())
    }

    fn transition(
        &self,
        id: &str,
        from: &[&str],
        to: &str,
        event_kind: &str,
        payload: Value,
        extra_sets: &str,
        extra_params: Vec<Box<dyn rusqlite::types::ToSql>>,
    ) -> Result<Task> {
        let conn = self.conn.lock().unwrap();
        // extra_sets placeholders occupy ?3.. first; the status IN-list
        // continues after them.
        let n_extra = extra_params.len();
        let placeholders: Vec<String> = (0..from.len())
            .map(|i| format!("?{}", 3 + n_extra + i))
            .collect();
        let sql = format!(
            "UPDATE tasks SET status = ?2 {} WHERE id = ?1 AND status IN ({})",
            extra_sets,
            placeholders.join(", ")
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(id.to_string()),
            Box::new(to.to_string()),
        ];
        params.extend(extra_params);
        for status in from {
            params.push(Box::new(status.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let updated = conn
            .execute(&sql, refs.as_slice())
            .map_err(db_error("transition"))?;
        drop(conn);
        if updated == 0 {
            let current = self.get_task(id)?;
            let current_status = current.map(|t| t.status).unwrap_or_else(|| "missing".into());
            return Err(AgentError::session(format!(
                "kanban: task {id} is '{current_status}' — cannot move to '{to}'"
            )));
        }
        self.append_event(id, event_kind, payload)?;
        self.get_task(id)?
            .ok_or_else(|| AgentError::session("kanban: task vanished mid-transition"))
    }

    /// todo → ready (hermes `ready_task`).
    pub fn ready_task(&self, id: &str) -> Result<Task> {
        self.transition(
            id,
            &["todo", "blocked"],
            "ready",
            "ready",
            serde_json::json!({}),
            "",
            vec![],
        )
    }

    pub fn assign_task(&self, id: &str, assignee: &str) -> Result<Task> {
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE tasks SET assignee = ?2 WHERE id = ?1 AND status NOT IN ('done', 'archived')",
                params![id, assignee],
            )
            .map_err(db_error("assign"))?;
        drop(conn);
        if updated == 0 {
            return Err(AgentError::session(format!(
                "kanban: task {id} not found or already terminal"
            )));
        }
        self.append_event(
            id,
            "assigned",
            serde_json::json!({ "assignee": assignee }),
        )?;
        self.get_task(id)?.ok_or_else(|| AgentError::session("kanban: task vanished"))
    }

    /// Atomically claim a ready task (or take over a running task whose
    /// claim expired) — hermes `claim_task` semantics.
    pub fn claim_task(&self, id: &str, claimer: &str, ttl_secs: i64) -> Result<Task> {
        let now = Self::now();
        let expires = now + ttl_secs;
        let lock = uuid::Uuid::new_v4().to_string();
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE tasks SET status = 'running', claim_lock = ?2, claim_expires = ?3, \
                 started_at = COALESCE(started_at, ?4), \
                 assignee = COALESCE(assignee, ?5) \
                 WHERE id = ?1 AND (status = 'ready' OR (status = 'running' AND claim_expires < ?4))",
                params![id, lock, expires, now, claimer],
            )
            .map_err(db_error("claim"))?;
        if updated == 0 {
            drop(conn);
            let task = self.get_task(id)?;
            return match task {
                Some(task)
                    if task.status == "running"
                        && task.claim_expires.map(|e| e >= now).unwrap_or(false) =>
                {
                    Err(AgentError::session(format!(
                        "kanban: task {id} already claimed by {} (expires in {}s)",
                        task.assignee.unwrap_or_else(|| "?".into()),
                        task.claim_expires.unwrap_or(now) - now
                    )))
                }
                Some(task) => Err(AgentError::session(format!(
                    "kanban: task {id} is '{}' — only ready tasks are claimable",
                    task.status
                ))),
                None => Err(AgentError::session(format!("kanban: task {id} not found"))),
            };
        }
        // One run row per attempt (hermes task_runs): recover any stale
        // active run, then open the new one.
        Self::recover_stale_run_conn(&conn, id, "invariant recovery on re-claim", now)?;
        let run_id = Self::start_run_conn(&conn, id, &lock, expires, now)?;
        drop(conn);
        self.append_event(
            id,
            "claimed",
            serde_json::json!({ "lock": lock, "expires": expires, "claimer": claimer, "run_id": run_id }),
        )?;
        self.get_task(id)?.ok_or_else(|| AgentError::session("kanban: task vanished"))
    }

    /// Extend a live claim (hermes `heartbeat_task`); only the lock
    /// holder may heartbeat.
    pub fn heartbeat_task(&self, id: &str, claimer: &str, ttl_secs: i64) -> Result<Task> {
        let now = Self::now();
        let expires = now + ttl_secs;
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE tasks SET last_heartbeat_at = ?2, claim_expires = ?3 \
                 WHERE id = ?1 AND status = 'running' AND (assignee = ?4 OR claim_lock IS NOT NULL)",
                params![id, now, expires, claimer],
            )
            .map_err(db_error("heartbeat"))?;
        if updated > 0 {
            conn.execute(
                "UPDATE task_runs SET last_heartbeat_at = ?2, claim_expires = ?3 \
                  WHERE id = (SELECT current_run_id FROM tasks WHERE id = ?1) \
                  AND ended_at IS NULL",
                params![id, now, expires],
            )
            .map_err(db_error("heartbeat run"))?;
        }
        drop(conn);
        if updated == 0 {
            return Err(AgentError::session(format!(
                "kanban: task {id} has no live claim to heartbeat"
            )));
        }
        self.get_task(id)?.ok_or_else(|| AgentError::session("kanban: task vanished"))
    }

    /// Move a task to done (hermes `complete_task` — allowed from any
    /// non-terminal status).
    pub fn complete_task(&self, id: &str, result: Option<&str>) -> Result<Task> {
        let now = Self::now();
        let task = self.transition(
            id,
            &["todo", "ready", "running", "scheduled", "blocked"],
            "done",
            "completed",
            serde_json::json!({ "result": result }),
            ", completed_at = ?3, result = ?4, claim_lock = NULL, claim_expires = NULL, \
             consecutive_failures = 0, last_failure_error = NULL",
            vec![Box::new(now), Box::new(result.map(|r| r.to_string()))],
        )?;
        match self.close_active_run(id, "done", "completed", result, None)? {
            Some(_) => {}
            None => {
                // CLI `kanban done` on a never-claimed task still leaves
                // an attempt-history row (hermes _synthesize_ended_run).
                self.synthesize_closed_run(id, "completed", result, None)?;
            }
        }
        Ok(task)
    }

    /// Block a task with a reason (hermes `block_task`).
    pub fn block_task(&self, id: &str, reason: &str) -> Result<Task> {
        let task = self.transition(
            id,
            &["todo", "ready", "running", "scheduled"],
            "blocked",
            "blocked",
            serde_json::json!({ "reason": reason }),
            ", claim_lock = NULL, claim_expires = NULL",
            vec![],
        )?;
        self.close_active_run(id, "blocked", "blocked", None, Some(reason))?;
        Ok(task)
    }

    /// blocked/scheduled → ready (hermes `unblock_task`). A deliberate
    /// unblock is a fresh start for the dispatcher's retry budget.
    pub fn unblock_task(&self, id: &str) -> Result<Task> {
        let task = self.transition(
            id,
            &["blocked", "scheduled"],
            "ready",
            "unblocked",
            serde_json::json!({}),
            ", consecutive_failures = 0, last_failure_error = NULL",
            vec![],
        )?;
        self.recover_stale_run(id, "invariant recovery on unblock")?;
        Ok(task)
    }

    /// Park a task in the scheduled column — waiting on time, not human
    /// input (hermes `kanban schedule`). The reason doubles as a comment.
    pub fn schedule_task(&self, id: &str, reason: &str) -> Result<Task> {
        let task = self.transition(
            id,
            &["todo", "ready", "blocked"],
            "scheduled",
            "scheduled",
            serde_json::json!({ "reason": reason }),
            "",
            vec![],
        )?;
        if !reason.trim().is_empty() {
            self.add_comment(id, "scheduler", reason.trim())?;
        }
        Ok(task)
    }

    /// Manually promote a todo/blocked task to ready (hermes `kanban
    /// promote`, the recovery path). Unless `force`, parents must all be
    /// done/archived — otherwise the dispatcher would demote the task
    /// again on the next tick.
    pub fn promote_task(&self, id: &str, reason: &str, force: bool) -> Result<Task> {
        if !force {
            let blocked_by: Vec<String> = self
                .parents_of(id)?
                .into_iter()
                .filter_map(|parent_id| match self.get_task(&parent_id) {
                    Ok(Some(parent))
                        if parent.status != "done" && parent.status != "archived" =>
                    {
                        Some(parent_id)
                    }
                    _ => None,
                })
                .collect();
            if !blocked_by.is_empty() {
                return Err(AgentError::session(format!(
                    "kanban: {} still has open parent(s) {} — use --force to promote anyway",
                    id,
                    blocked_by.join(", ")
                )));
            }
        }
        self.transition(
            id,
            &["todo", "blocked"],
            "ready",
            "promoted",
            serde_json::json!({ "reason": reason, "force": force }),
            "",
            vec![],
        )
    }

    /// Release an active worker claim on a running task without completing
    /// it (hermes `kanban reclaim`): back to ready for a fresh worker.
    pub fn reclaim_task(&self, id: &str, reason: &str) -> Result<Task> {
        let task = self.transition(
            id,
            &["running"],
            "ready",
            "reclaimed",
            serde_json::json!({ "reason": reason }),
            ", claim_lock = NULL, claim_expires = NULL, worker_pid = NULL,              last_heartbeat_at = NULL",
            vec![],
        )?;
        self.close_active_run(id, "reclaimed", "reclaimed", Some(reason), None)?;
        Ok(task)
    }

    /// Close any stale active run as `reclaimed` (hermes invariant
    /// recovery, public wrapper for non-transition call sites).
    pub fn recover_stale_run(&self, task_id: &str, note: &str) -> Result<()> {
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        Self::recover_stale_run_conn(&conn, task_id, note, now)
    }

    /// Reassign a task, optionally reclaiming an active claim first
    /// (hermes `kanban reassign`). `assignee` "none"/"" clears it.
    pub fn reassign_task(
        &self,
        id: &str,
        assignee: &str,
        reclaim_first: bool,
        reason: &str,
    ) -> Result<Task> {
        let task = self
            .get_task(id)?
            .ok_or_else(|| AgentError::session(format!("kanban: task {id} not found")))?;
        if reclaim_first && task.status == "running" {
            self.reclaim_task(id, reason)?;
        }
        let target = if assignee.eq_ignore_ascii_case("none") || assignee.trim().is_empty() {
            None
        } else {
            Some(assignee.trim().to_string())
        };
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE tasks SET assignee = ?2 WHERE id = ?1 AND status NOT IN ('done', 'archived')",
                params![id, target],
            )
            .map_err(db_error("reassign"))?;
        drop(conn);
        if updated == 0 {
            return Err(AgentError::session(format!(
                "kanban: task {id} not found or already terminal"
            )));
        }
        self.append_event(
            id,
            "reassigned",
            serde_json::json!({ "assignee": target, "reason": reason }),
        )?;
        self.get_task(id)?.ok_or_else(|| AgentError::session("kanban: task vanished"))
    }

    /// Edit a task's title/body (hermes `kanban edit`).
    pub fn edit_task(
        &self,
        id: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<Task> {
        if let Some(candidate) = title {
            if candidate.trim().is_empty() {
                return Err(AgentError::session("kanban: title cannot be blank"));
            }
        }
        if title.is_none() && body.is_none() {
            return Err(AgentError::session("kanban: nothing to edit"));
        }
        let conn = self.conn.lock().unwrap();
        let mut sets: Vec<String> = Vec::new();
        if title.is_some() {
            sets.push("title = ?2".to_string());
        }
        if body.is_some() {
            sets.push(format!("body = ?{}", if title.is_some() { 3 } else { 2 }));
        }
        let sql = format!(
            "UPDATE tasks SET {} WHERE id = ?1 AND status NOT IN ('archived')",
            sets.join(", ")
        );
        let updated = match (title, body) {
            (Some(title), Some(body)) => conn.execute(&sql, params![id, title.trim(), body]),
            (Some(title), None) => conn.execute(&sql, params![id, title.trim()]),
            (None, Some(body)) => conn.execute(&sql, params![id, body]),
            (None, None) => unreachable!("checked above"),
        }
        .map_err(db_error("edit"))?;
        drop(conn);
        if updated == 0 {
            return Err(AgentError::session(format!(
                "kanban: task {id} not found or archived"
            )));
        }
        self.append_event(id, "edited", serde_json::json!({}))?;
        self.get_task(id)?.ok_or_else(|| AgentError::session("kanban: task vanished"))
    }

    /// Per-task model override (hermes `kanban set-model`).
    pub fn set_model(&self, id: &str, model: Option<&str>) -> Result<Task> {
        let model = model.map(str::trim).filter(|m| !m.is_empty());
        let conn = self.conn.lock().unwrap();
        let updated = conn
            .execute(
                "UPDATE tasks SET model = ?2 WHERE id = ?1 AND status NOT IN ('done', 'archived')",
                params![id, model],
            )
            .map_err(db_error("set-model"))?;
        drop(conn);
        if updated == 0 {
            return Err(AgentError::session(format!(
                "kanban: task {id} not found or already terminal"
            )));
        }
        self.append_event(
            id,
            "model_set",
            serde_json::json!({ "model": model }),
        )?;
        self.get_task(id)?.ok_or_else(|| AgentError::session("kanban: task vanished"))
    }

    pub fn archive_task(&self, id: &str) -> Result<Task> {
        self.transition(
            id,
            &["todo", "ready", "running", "scheduled", "blocked", "done"],
            "archived",
            "archived",
            serde_json::json!({}),
            ", claim_lock = NULL, claim_expires = NULL",
            vec![],
        )
    }

    // ------------------------------------------------------------- comments

    pub fn add_comment(&self, task_id: &str, author: &str, body: &str) -> Result<()> {
        if self.get_task(task_id)?.is_none() {
            return Err(AgentError::session(format!(
                "kanban: task {task_id} not found"
            )));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO task_comments (task_id, author, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, author, body, Self::now()],
        )
        .map_err(db_error("comment"))?;
        Ok(())
    }

    pub fn comments(&self, task_id: &str) -> Result<Vec<Comment>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, author, body, created_at FROM task_comments \
                 WHERE task_id = ?1 ORDER BY created_at ASC, id ASC",
            )
            .map_err(db_error("comments"))?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    author: row.get(2)?,
                    body: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(db_error("comments"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("comments"))
    }

    pub fn events(&self, task_id: &str) -> Result<Vec<TaskEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, kind, payload, created_at FROM task_events \
                 WHERE task_id = ?1 ORDER BY created_at ASC, id ASC",
            )
            .map_err(db_error("events"))?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                let payload: String = row.get(3)?;
                Ok(TaskEvent {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                    created_at: row.get(4)?,
                })
            })
            .map_err(db_error("events"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("events"))
    }

    /// Link two tasks parent → child (hermes `link_tasks`). Idempotent;
    /// both tasks must exist; self-links and exact duplicates are no-ops.
    pub fn link_tasks(&self, parent_id: &str, child_id: &str) -> Result<()> {
        if parent_id == child_id {
            return Err(AgentError::session("kanban: cannot link a task to itself"));
        }
        if self.get_task(parent_id)?.is_none() {
            return Err(AgentError::session(format!(
                "kanban: parent task {parent_id} not found"
            )));
        }
        if self.get_task(child_id)?.is_none() {
            return Err(AgentError::session(format!(
                "kanban: child task {child_id} not found"
            )));
        }
        let conn = self.conn.lock().unwrap();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO task_links (parent_id, child_id, created_at)                  VALUES (?1, ?2, ?3)",
                params![parent_id, child_id, Self::now()],
            )
            .map_err(db_error("link"))?;
        drop(conn);
        if inserted > 0 {
            self.append_event(
                child_id,
                "linked",
                serde_json::json!({ "parent_id": parent_id }),
            )?;
        }
        Ok(())
    }

    /// Remove a parent → child link (hermes `unlink_tasks`). Idempotent.
    pub fn unlink_tasks(&self, parent_id: &str, child_id: &str) -> Result<()> {
        let removed = self
            .conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM task_links WHERE parent_id = ?1 AND child_id = ?2",
                params![parent_id, child_id],
            )
            .map_err(db_error("unlink"))?;
        if removed > 0 {
            self.append_event(
                child_id,
                "unlinked",
                serde_json::json!({ "parent_id": parent_id }),
            )?;
        }
        Ok(())
    }

    /// Parent task ids of `task_id`, oldest link first.
    pub fn parents_of(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT parent_id FROM task_links WHERE child_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(db_error("parents"))?;
        let rows = stmt
            .query_map(params![task_id], |row| row.get::<_, String>(0))
            .map_err(db_error("parents"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("parents"))
    }

    /// Attach a file path or URL to a task (hermes task_attachments).
    pub fn attach(&self, task_id: &str, kind: &str, value: &str) -> Result<()> {
        if self.get_task(task_id)?.is_none() {
            return Err(AgentError::session(format!(
                "kanban: task {task_id} not found"
            )));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO task_attachments (task_id, kind, value, created_at)              VALUES (?1, ?2, ?3, ?4)",
            params![task_id, kind, value, Self::now()],
        )
        .map_err(db_error("attach"))?;
        Ok(())
    }

    /// Attachments of `task_id` as (kind, value) pairs, oldest first.
    pub fn attachments(&self, task_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT kind, value FROM task_attachments WHERE task_id = ?1 ORDER BY id ASC",
            )
            .map_err(db_error("attachments"))?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error("attachments"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("attachments"))
    }

    /// Attachments of `task_id` as (id, kind, value) rows, oldest first.
    pub fn attachments_with_ids(&self, task_id: &str) -> Result<Vec<(i64, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, value FROM task_attachments WHERE task_id = ?1 ORDER BY id ASC",
            )
            .map_err(db_error("attachments"))?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(db_error("attachments"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("attachments"))
    }

    /// Delete one attachment by id (hermes `kanban attach-rm`).
    pub fn remove_attachment(&self, attachment_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let removed = conn
            .execute(
                "DELETE FROM task_attachments WHERE id = ?1",
                params![attachment_id],
            )
            .map_err(db_error("attach-rm"))?;
        Ok(removed > 0)
    }

    // ------------------------------------------------------------------
    // Notification subscriptions (hermes kanban_notify_subs)
    // ------------------------------------------------------------------

    /// Subscribe a gateway chat to a task's terminal events (hermes
    /// `add_notify_sub`). Idempotent: duplicate subscribes refresh
    /// `chat_type` / `notifier_profile` / `delivery_metadata` when the
    /// existing value is unset (or the metadata changed). New subs start
    /// "caught up": `last_event_id` snaps to the task's current max
    /// event id so subscribers never replay history.
    pub fn add_notify_sub(&self, task_id: &str, spec: &NewNotifySub<'_>) -> Result<()> {
        if self.get_task(task_id)?.is_none() {
            return Err(AgentError::session(format!(
                "kanban: no such task: {task_id}"
            )));
        }
        let thread_id = spec.thread_id.unwrap_or("");
        let metadata_json = spec
            .delivery_metadata
            .as_ref()
            .map(|v| v.to_string())
            .filter(|s| s != "null");
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO kanban_notify_subs
                (task_id, platform, chat_id, chat_type, thread_id, user_id,
                 notifier_profile, delivery_metadata, created_at, last_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                     COALESCE((SELECT MAX(id) FROM task_events WHERE task_id = ?1), 0))",
            params![
                task_id,
                spec.platform,
                spec.chat_id,
                spec.chat_type,
                thread_id,
                spec.user_id,
                spec.notifier_profile,
                metadata_json,
                now,
            ],
        )
        .map_err(db_error("notify-subscribe"))?;
        if let Some(chat_type) = spec.chat_type {
            // Self-heal rows created before chat_type was persisted.
            conn.execute(
                "UPDATE kanban_notify_subs SET chat_type = ?1
                 WHERE task_id = ?2 AND platform = ?3 AND chat_id = ?4 AND thread_id = ?5
                   AND (chat_type IS NULL OR chat_type = '')",
                params![chat_type, task_id, spec.platform, spec.chat_id, thread_id],
            )
            .map_err(db_error("notify-subscribe chat_type"))?;
        }
        if let Some(profile) = spec.notifier_profile {
            // Self-heal legacy rows that predate notifier ownership.
            conn.execute(
                "UPDATE kanban_notify_subs SET notifier_profile = ?1
                 WHERE task_id = ?2 AND platform = ?3 AND chat_id = ?4 AND thread_id = ?5
                   AND (notifier_profile IS NULL OR notifier_profile = '')",
                params![profile, task_id, spec.platform, spec.chat_id, thread_id],
            )
            .map_err(db_error("notify-subscribe profile"))?;
        }
        if let Some(meta) = &metadata_json {
            // Duplicate subscribes refresh the routing anchor.
            conn.execute(
                "UPDATE kanban_notify_subs SET delivery_metadata = ?1
                 WHERE task_id = ?2 AND platform = ?3 AND chat_id = ?4 AND thread_id = ?5",
                params![meta, task_id, spec.platform, spec.chat_id, thread_id],
            )
            .map_err(db_error("notify-subscribe metadata"))?;
        }
        Ok(())
    }

    /// List notification subscriptions, optionally for one task (hermes
    /// `list_notify_subs`).
    pub fn list_notify_subs(&self, task_id: Option<&str>) -> Result<Vec<NotifySub>> {
        let conn = self.conn.lock().unwrap();
        let (sql, params_box): (String, Vec<Box<dyn rusqlite::ToSql>>) = match task_id {
            Some(id) => (
                "SELECT task_id, platform, chat_id, chat_type, thread_id, user_id,
                        notifier_profile, delivery_metadata, created_at, last_event_id
                 FROM kanban_notify_subs WHERE task_id = ?1
                 ORDER BY created_at ASC".into(),
                vec![Box::new(id.to_string())],
            ),
            None => (
                "SELECT task_id, platform, chat_id, chat_type, thread_id, user_id,
                        notifier_profile, delivery_metadata, created_at, last_event_id
                 FROM kanban_notify_subs ORDER BY created_at ASC".into(),
                Vec::new(),
            ),
        };
        let mut stmt = conn.prepare(&sql).map_err(db_error("notify-list"))?;
        let refs: Vec<&dyn rusqlite::ToSql> = params_box.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let metadata: Option<String> = row.get(7)?;
                Ok(NotifySub {
                    task_id: row.get(0)?,
                    platform: row.get(1)?,
                    chat_id: row.get(2)?,
                    chat_type: row.get(3)?,
                    thread_id: row.get(4)?,
                    user_id: row.get(5)?,
                    notifier_profile: row.get(6)?,
                    delivery_metadata: metadata
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .and_then(|s| serde_json::from_str(s).ok()),
                    created_at: row.get(8)?,
                    last_event_id: row.get(9)?,
                })
            })
            .map_err(db_error("notify-list"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("notify-list"))
    }

    /// Remove one subscription; returns whether a row existed (hermes
    /// `remove_notify_sub`).
    pub fn remove_notify_sub(
        &self,
        task_id: &str,
        platform: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let removed = conn
            .execute(
                "DELETE FROM kanban_notify_subs WHERE task_id = ?1
                 AND platform = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![task_id, platform, chat_id, thread_id.unwrap_or("")],
            )
            .map_err(db_error("notify-unsubscribe"))?;
        Ok(removed > 0)
    }

    /// Advance a subscription's delivery cursor (hermes
    /// `advance_notify_cursor`). Called by the gateway notifier after
    /// events were successfully delivered.
    pub fn advance_notify_cursor(
        &self,
        task_id: &str,
        platform: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        new_cursor: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE kanban_notify_subs SET last_event_id = ?1
             WHERE task_id = ?2 AND platform = ?3 AND chat_id = ?4 AND thread_id = ?5",
            params![new_cursor, task_id, platform, chat_id, thread_id.unwrap_or("")],
        )
        .map_err(db_error("notify-cursor"))?;
        Ok(())
    }

    /// `(new_cursor, events)` unseen by one subscription (hermes
    /// `unseen_events_for_sub`). Only events with `id > last_event_id`;
    /// the cursor is NOT advanced here — call [`Self::advance_notify_cursor`]
    /// after successful delivery.
    pub fn unseen_events_for_sub(
        &self,
        task_id: &str,
        platform: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        kinds: Option<&[&str]>,
    ) -> Result<(i64, Vec<TaskEvent>)> {
        let conn = self.conn.lock().unwrap();
        let cursor: Option<i64> = conn
            .query_row(
                "SELECT last_event_id FROM kanban_notify_subs
                 WHERE task_id = ?1 AND platform = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![task_id, platform, chat_id, thread_id.unwrap_or("")],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error("notify-unseen"))?;
        let Some(cursor) = cursor else {
            return Ok((0, Vec::new()));
        };
        let mut sql = String::from(
            "SELECT id, task_id, kind, payload, created_at FROM task_events
             WHERE task_id = ?1 AND id > ?2 ",
        );
        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(task_id.to_string()),
            Box::new(cursor),
        ];
        if let Some(kinds) = kinds.filter(|k| !k.is_empty()) {
            sql.push_str("AND kind IN (");
            for (i, _) in kinds.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format!("?{}", i + 3));
                param_values.push(Box::new(kinds[i].to_string()));
            }
            sql.push_str(") ");
        }
        sql.push_str("ORDER BY id ASC");
        let mut stmt = conn.prepare(&sql).map_err(db_error("notify-unseen"))?;
        let refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let payload: String = row.get(3)?;
                Ok(TaskEvent {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                    created_at: row.get(4)?,
                })
            })
            .map_err(db_error("notify-unseen"))?;
        let events: Vec<TaskEvent> = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error("notify-unseen"))?;
        let max_id = events.last().map(|e| e.id).unwrap_or(cursor).max(cursor);
        Ok((max_id, events))
    }

    // ------------------------------------------------------------------
    // Run attempt history (hermes task_runs)
    // ------------------------------------------------------------------

    /// Start a `running` run row for a freshly claimed task and point
    /// `tasks.current_run_id` at it (hermes claim-time INSERT INTO
    /// task_runs). Caller must hold `conn`.
    fn start_run_conn(
        conn: &Connection,
        task_id: &str,
        lock: &str,
        expires: i64,
        now: i64,
    ) -> Result<i64> {
        let (assignee, max_runtime): (Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT assignee, max_runtime_seconds FROM tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error("run start"))?;
        conn.execute(
            "INSERT INTO task_runs (
                task_id, profile, status, claim_lock, claim_expires,
                max_runtime_seconds, started_at
             ) VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6)",
            params![task_id, assignee, lock, expires, max_runtime, now],
        )
        .map_err(db_error("run start"))?;
        let run_id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE tasks SET current_run_id = ?2 WHERE id = ?1",
            params![task_id, run_id],
        )
        .map_err(db_error("run start pointer"))?;
        Ok(run_id)
    }

    /// Close any still-open run left over from a previous attempt as
    /// `reclaimed` (hermes invariant recovery on re-claim / unblock).
    /// Caller must hold `conn`.
    fn recover_stale_run_conn(conn: &Connection, task_id: &str, note: &str, now: i64) -> Result<()> {
        conn.execute(
            "UPDATE task_runs
                SET status = 'reclaimed', outcome = 'reclaimed',
                    summary = COALESCE(summary, ?2),
                    ended_at = ?3,
                    claim_lock = NULL, claim_expires = NULL, worker_pid = NULL
              WHERE id = (SELECT current_run_id FROM tasks WHERE id = ?1)
                AND ended_at IS NULL",
            params![task_id, note, now],
        )
        .map_err(db_error("run recover"))?;
        conn.execute(
            "UPDATE tasks SET current_run_id = NULL WHERE id = ?1",
            params![task_id],
        )
        .map_err(db_error("run recover pointer"))?;
        Ok(())
    }

    /// Close the task's active run (hermes `close_active_run`). Returns
    /// the closed run id, or `None` when no run was active (e.g. a CLI
    /// `done` on a never-claimed task). `summary` / `error` keep any
    /// existing value when `None`.
    pub fn close_active_run(
        &self,
        task_id: &str,
        status: &str,
        outcome: &str,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<Option<i64>> {
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        let run_id: Option<i64> = conn
            .query_row(
                "SELECT current_run_id FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error("run close"))?
            .flatten();
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE task_runs
                SET status = ?2, outcome = ?3,
                    summary = COALESCE(?4, summary),
                    error = COALESCE(?5, error),
                    ended_at = ?6,
                    claim_lock = NULL, claim_expires = NULL, worker_pid = NULL
              WHERE id = ?1 AND ended_at IS NULL",
            params![run_id, status, outcome, summary, error, now],
        )
        .map_err(db_error("run close"))?;
        conn.execute(
            "UPDATE tasks SET current_run_id = NULL WHERE id = ?1",
            params![task_id],
        )
        .map_err(db_error("run close pointer"))?;
        drop(conn);
        Ok(Some(run_id))
    }

    /// Record an "instant" closed run for an attempt that never held a
    /// claim — CLI completes and dispatcher spawn failures (hermes
    /// `_synthesize_ended_run`).
    pub fn synthesize_closed_run(
        &self,
        task_id: &str,
        outcome: &str,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<i64> {
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        let profile: Option<String> = conn
            .query_row(
                "SELECT assignee FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error("run synthesize"))?
            .flatten();
        conn.execute(
            "INSERT INTO task_runs (
                task_id, profile, status, outcome, summary, error,
                started_at, ended_at
             ) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?6)",
            params![task_id, profile, outcome, summary, error, now],
        )
        .map_err(db_error("run synthesize"))?;
        Ok(conn.last_insert_rowid())
    }

    /// Attempt history for a task, oldest first (hermes `list_runs`).
    /// `state_type` is `status` or `outcome` and must pair with
    /// `state_name`; `include_active=false` drops the running attempt.
    pub fn list_runs(
        &self,
        task_id: &str,
        include_active: bool,
        state_type: Option<&str>,
        state_name: Option<&str>,
    ) -> Result<Vec<Run>> {
        match (state_type, state_name) {
            (None, None) => {}
            (Some(t), Some(_)) if t == "status" || t == "outcome" => {}
            (Some(_), Some(_)) => {
                return Err(AgentError::session(
                    "kanban runs: state-type must be 'status' or 'outcome'",
                ));
            }
            _ => {
                return Err(AgentError::session(
                    "kanban runs: pass both --state-type and --state-name, or omit both",
                ));
            }
        }
        let mut sql = String::from(
            "SELECT id, task_id, profile, step_key, status, claim_lock, claim_expires,
                   worker_pid, max_runtime_seconds, last_heartbeat_at, started_at,
                   ended_at, outcome, summary, metadata, error
              FROM task_runs WHERE task_id = ?1 ",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(task_id.to_string())];
        if !include_active {
            sql.push_str("AND ended_at IS NOT NULL ");
        }
        if let (Some(state_type), Some(state_name)) = (state_type, state_name) {
            sql.push_str(&format!("AND {state_type} = ?2 "));
            param_values.push(Box::new(state_name.to_string()));
        }
        sql.push_str("ORDER BY started_at ASC, id ASC");
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql).map_err(db_error("runs"))?;
        let refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), run_from_row)
            .map_err(db_error("runs"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("runs"))
    }

    /// Most recent run regardless of outcome, active or closed (hermes
    /// `latest_run`).
    pub fn latest_run(&self, task_id: &str) -> Result<Option<Run>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, profile, step_key, status, claim_lock, claim_expires,
                       worker_pid, max_runtime_seconds, last_heartbeat_at, started_at,
                       ended_at, outcome, summary, metadata, error
                  FROM task_runs WHERE task_id = ?1
                 ORDER BY started_at DESC, id DESC LIMIT 1",
            )
            .map_err(db_error("latest run"))?;
        let mut rows = stmt
            .query_map(params![task_id], run_from_row)
            .map_err(db_error("latest run"))?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(db_error("latest run"))?)),
            None => Ok(None),
        }
    }

    /// Latest non-null run summary for a task (hermes `latest_summary`)
    /// — the worker's handoff, surfaced when `tasks.result` is empty.
    pub fn latest_summary(&self, task_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let summary: Option<String> = conn
            .query_row(
                "SELECT summary FROM task_runs
                  WHERE task_id = ?1 AND summary IS NOT NULL
                  ORDER BY COALESCE(ended_at, 0) DESC, id DESC LIMIT 1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error("latest summary"))?
            .flatten();
        Ok(summary)
    }

    /// Full text a worker reads to understand its task (hermes
    /// `build_worker_context`): header, capped body, attachments, prior
    /// attempts (run summaries/errors/metadata, capped), done-parent
    /// handoffs, assignee's recent completed runs elsewhere, comment
    /// thread. Caps keep prompts bounded on pathological boards.
    pub fn build_worker_context(&self, task_id: &str) -> Result<String> {
        let task = self
            .get_task(task_id)?
            .ok_or_else(|| AgentError::session(format!("kanban: unknown task {task_id}")))?;
        let now = Self::now();
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("# Kanban task {}: {}", task.id, task.title));
        lines.push(String::new());
        lines.push(format!(
            "Assignee: {}",
            task.assignee.as_deref().unwrap_or("(unassigned)")
        ));
        lines.push(format!("Status:   {}", task.status));
        if let Some(max_runtime) = task.max_runtime_seconds {
            lines.push(format!("Max runtime: {max_runtime}s"));
        }
        lines.push(String::new());

        if !task.body.trim().is_empty() {
            lines.push("## Body".into());
            lines.push(cap_field(&task.body, CTX_MAX_BODY_BYTES));
            lines.push(String::new());
        }

        let attachments = self.attachments(task_id).unwrap_or_default();
        if !attachments.is_empty() {
            lines.push("## Attachments".into());
            lines.push(
                "Files/links attached to this task. Read them with the file/terminal                  tools:"
                    .into(),
            );
            for (kind, value) in &attachments {
                lines.push(format!("- `{kind}` → `{value}`"));
            }
            lines.push(String::new());
        }

        // Prior attempts: closed runs only (the active run is this worker).
        let all_prior: Vec<Run> = self
            .list_runs(task_id, false, None, None)?
            .into_iter()
            .filter(|r| r.ended_at.is_some())
            .collect();
        let (omitted, shown, first_idx) = if all_prior.len() > CTX_MAX_PRIOR_ATTEMPTS {
            let omitted = all_prior.len() - CTX_MAX_PRIOR_ATTEMPTS;
            (omitted, all_prior[omitted..].to_vec(), omitted + 1)
        } else {
            (0, all_prior.clone(), 1)
        };
        if !shown.is_empty() {
            lines.push("## Prior attempts on this task".into());
            if omitted > 0 {
                lines.push(format!(
                    "_({} earlier attempt{} omitted; showing most recent {})_",
                    omitted,
                    if omitted != 1 { "s" } else { "" },
                    shown.len()
                ));
            }
            for (offset, run) in shown.iter().enumerate() {
                let idx = first_idx + offset;
                let profile = run.profile.as_deref().unwrap_or("(unknown)");
                let outcome = run.outcome.as_deref().unwrap_or(&run.status);
                lines.push(format!(
                    "### Attempt {idx} — {outcome} ({profile}, {})",
                    ctx_timestamp(run.started_at, now)
                ));
                if let Some(summary) = run.summary.as_deref().filter(|s| !s.trim().is_empty()) {
                    lines.push(cap_field(summary, CTX_MAX_FIELD_BYTES));
                }
                if let Some(err) = run.error.as_deref().filter(|s| !s.trim().is_empty()) {
                    lines.push(format!("_error_: {}", cap_field(err, CTX_MAX_FIELD_BYTES)));
                }
                if let Some(meta) = &run.metadata {
                    lines.push(format!("_metadata_: `{}`", cap_field(&meta.to_string(), CTX_MAX_FIELD_BYTES)));
                }
                lines.push(String::new());
            }
        }

        // Done-parent handoffs: prefer the latest completed run's
        // summary, fall back to task.result for legacy rows.
        let parent_ids = self.parents_of(task_id)?;
        if !parent_ids.is_empty() {
            let mut wrote_header = false;
            for parent_id in parent_ids {
                let Some(parent) = self.get_task(&parent_id)? else {
                    continue;
                };
                if parent.status != "done" {
                    continue;
                }
                let completed_run = self
                    .list_runs(&parent_id, true, Some("outcome"), Some("completed"))?
                    .into_iter()
                    .max_by_key(|r| r.started_at);
                if !wrote_header {
                    lines.push("## Parent task results".into());
                    lines.push(
                        "_Handoffs from upstream tasks, captured when each parent completed                          (see age below). These are point-in-time snapshots, not live state —                          if a result drives your current work and it's not recent, re-verify                          against the source before acting on it as current._"
                            .into(),
                    );
                    wrote_header = true;
                }
                let done_ts = completed_run
                    .as_ref()
                    .and_then(|r| r.ended_at)
                    .or(parent.completed_at);
                let age_suffix = done_ts
                    .map(|ts| format!(" (completed {})", relative_age(ts, now)))
                    .unwrap_or_default();
                lines.push(format!("### {parent_id}{age_suffix}"));
                let body = completed_run
                    .as_ref()
                    .and_then(|r| r.summary.as_deref())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| cap_field(s, CTX_MAX_FIELD_BYTES))
                    .or_else(|| {
                        parent
                            .result
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| cap_field(s, CTX_MAX_FIELD_BYTES))
                    })
                    .unwrap_or_else(|| "(no result recorded)".into());
                lines.push(body);
                if let Some(run) = completed_run.as_ref().filter(|r| r.metadata.is_some()) {
                    let meta = run.metadata.as_ref().unwrap();
                    lines.push(format!(
                        "_metadata_: `{}`",
                        cap_field(&meta.to_string(), CTX_MAX_FIELD_BYTES)
                    ));
                }
                lines.push(String::new());
            }
        }

        // Cross-task role history: the assignee's 5 most recent completed
        // runs elsewhere (implicit continuity, hermes role history).
        if let Some(assignee) = task.assignee.as_deref().filter(|a| !a.is_empty()) {
            let role_rows: Vec<(String, String, Option<String>, i64)> = {
                let conn = self.conn.lock().unwrap();
                let mut stmt = conn
                    .prepare(
                        "SELECT t.id, t.title, r.summary, r.ended_at
                           FROM task_runs r JOIN tasks t ON r.task_id = t.id
                          WHERE r.profile = ?1 AND r.task_id != ?2
                            AND r.outcome = 'completed'
                          ORDER BY r.ended_at DESC LIMIT 5",
                    )
                    .map_err(db_error("context roles"))?;
                let rows = stmt
                    .query_map(params![assignee, task_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })
                    .map_err(db_error("context roles"))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_error("context roles"))?
            };
            if !role_rows.is_empty() {
                lines.push(format!("## Recent work by @{assignee}"));
                for (id, title, summary, ended_at) in role_rows {
                    let first = summary
                        .as_deref()
                        .map(|s| s.trim().lines().next().unwrap_or(""))
                        .unwrap_or("");
                    let first: String = first.chars().take(200).collect();
                    let first = if first.is_empty() { "(no summary)".into() } else { first };
                    lines.push(format!(
                        "- {id} — {title} ({}): {first}",
                        ctx_timestamp(ended_at, now)
                    ));
                }
                lines.push(String::new());
            }
        }

        // Comment thread, capped (comment-storm protection).
        let all_comments = self.comments(task_id)?;
        let (omitted_c, shown_c) = if all_comments.len() > CTX_MAX_COMMENTS {
            let omitted = all_comments.len() - CTX_MAX_COMMENTS;
            (omitted, all_comments[omitted..].to_vec())
        } else {
            (0, all_comments.clone())
        };
        if !shown_c.is_empty() {
            lines.push("## Comment thread".into());
            if omitted_c > 0 {
                lines.push(format!(
                    "_({} earlier comment{} omitted; showing most recent {})_",
                    omitted_c,
                    if omitted_c != 1 { "s" } else { "" },
                    shown_c.len()
                ));
            }
            for comment in &shown_c {
                // Explicit "comment from worker" framing: operator/system
                // author names must not read as system directives (hermes
                // #22452 defense-in-depth).
                let safe_author = comment.author.replace('`', "");
                lines.push(format!(
                    "comment from worker `{safe_author}` at {}:",
                    ctx_timestamp(comment.created_at, now)
                ));
                lines.push(cap_field(&comment.body, CTX_MAX_COMMENT_BYTES));
                lines.push(String::new());
            }
        }

        let mut out = lines.join("\n");
        while out.ends_with("\n\n") {
            out.pop();
        }
        out.push('\n');
        Ok(out)
    }

    /// Child task ids of `task_id`, oldest link first.
    pub fn children_of(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT child_id FROM task_links WHERE parent_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(db_error("children"))?;
        let rows = stmt
            .query_map(params![task_id], |row| row.get::<_, String>(0))
            .map_err(db_error("children"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("children"))
    }

    // ------------------------------------------------------------------
    // Dispatcher tick (hermes kanban_db.dispatch_once semantics, scoped:
    // reclaim stale claims, promote parent-done todos, spawn ready tasks)
    // ------------------------------------------------------------------

    /// Reset `running` tasks whose claim TTL expired back to `ready`
    /// (hermes `release_stale_claims`). A stale claim whose worker pid is
    /// still alive gets extended instead of reclaimed, so a slow-but-healthy
    /// worker is not yanked mid-flight. Returns reclaimed task ids.
    /// Kill + requeue running workers that exceeded their per-attempt
    /// runtime cap (hermes `reap_timed_out`): SIGTERM with a 5 s grace,
    /// SIGKILL after, task back to `ready` with a `timed_out` event so the
    /// next tick re-spawns it. Returns reaped task ids.
    pub fn reap_timed_out(&self) -> Result<Vec<String>> {
        let now = Self::now();
        let candidates: Vec<(String, Option<i64>, i64, i64)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, worker_pid, started_at, max_runtime_seconds FROM tasks \
                     WHERE status = 'running' AND max_runtime_seconds IS NOT NULL \
                     AND started_at IS NOT NULL",
                )
                .map_err(db_error("reap"))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(db_error("reap"))?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("reap"))?
        };
        let mut reaped = Vec::new();
        for (id, worker_pid, started_at, limit) in candidates {
            let elapsed = now - started_at;
            if elapsed < limit {
                continue;
            }
            let mut sigkill = false;
            if let Some(pid) = worker_pid {
                if pid_alive(pid) {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                    for _ in 0..10 {
                        if !pid_alive(pid) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                    if pid_alive(pid) {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                        sigkill = true;
                    }
                }
            }
            self.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE tasks SET status = 'ready', claim_lock = NULL, \
                     claim_expires = NULL, worker_pid = NULL, last_heartbeat_at = NULL \
                     WHERE id = ?1 AND status = 'running'",
                    params![id],
                )
                .map_err(db_error("reap release"))?;
            self.append_event(
                &id,
                "timed_out",
                serde_json::json!({
                    "elapsed_seconds": elapsed,
                    "limit_seconds": limit,
                    "sigkill": sigkill,
                }),
            )?;
            self.close_active_run(&id, "timed_out", "timed_out", None, None)?;
            // Unified failure accounting: a timed-out attempt consumes
            // the retry budget and may trip the breaker (ready → blocked
            // + gave_up) — hermes _record_task_failure on the timeout
            // path.
            self.record_task_failure(
                &id,
                &format!("elapsed {elapsed}s > limit {limit}s"),
                "timed_out",
                2,
            )?;
            reaped.push(id);
        }
        Ok(reaped)
    }

    /// Crash-detection grace period (hermes
    /// `DEFAULT_CRASH_GRACE_SECONDS` = 30, overridable via
    /// `ULNCLAW_KANBAN_CRASH_GRACE_SECONDS`; 0 restores immediate
    /// reclaim for tests). A freshly spawned worker must not be
    /// reclaimed before its pid is visible on /proc.
    fn crash_grace_seconds() -> i64 {
        std::env::var("ULNCLAW_KANBAN_CRASH_GRACE_SECONDS")
            .ok()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .filter(|v| *v >= 0)
            .unwrap_or(30)
    }

    /// Reclaim `running` tasks whose worker pid is no longer alive
    /// (hermes `detect_crashed_workers`) — immediate liveness check
    /// instead of waiting out the claim TTL. Emits a `crashed` event,
    /// closes the active run with outcome `crashed`, and counts the
    /// failure against the circuit breaker.
    pub fn detect_crashed_workers(&self) -> Result<Vec<String>> {
        let now = Self::now();
        let grace = Self::crash_grace_seconds();
        let rows: Vec<(String, i64, Option<i64>)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, worker_pid, started_at FROM tasks \
                     WHERE status = 'running' AND worker_pid IS NOT NULL",
                )
                .map_err(db_error("crashed"))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                })
                .map_err(db_error("crashed"))?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("crashed"))?
        };
        let mut crashed = Vec::new();
        for (id, pid, started_at) in rows {
            if let Some(started) = started_at {
                if now - started < grace {
                    continue;
                }
            }
            if pid_alive(pid) {
                continue;
            }
            let updated = self
                .conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE tasks SET status = 'ready', claim_lock = NULL, \
                     claim_expires = NULL, worker_pid = NULL \
                     WHERE id = ?1 AND status = 'running' AND worker_pid = ?2",
                    params![id, pid],
                )
                .map_err(db_error("crashed release"))?;
            if updated != 1 {
                continue;
            }
            let error = format!("pid {pid} not alive");
            self.close_active_run(&id, "crashed", "crashed", None, Some(&error))?;
            self.append_event(&id, "crashed", serde_json::json!({ "pid": pid }))?;
            // Unified failure accounting — a crash consumes the retry
            // budget and may trip the breaker (ready → blocked +
            // gave_up).
            self.record_task_failure(&id, &error, "crashed", 2)?;
            crashed.push(id);
        }
        Ok(crashed)
    }

    /// Reclaim `running` tasks that show no progress: running longer
    /// than `stale_timeout_seconds` AND no heartbeat within the last
    /// hour (or never) — hermes `detect_stale_running`. The worker is
    /// terminated (SIGTERM then SIGKILL); the run closes with outcome
    /// `stale` and a `stale` event is emitted. Deliberately NOT counted
    /// as a failure (hermes policy): long tasks without heartbeats are
    /// not worker failures. `stale_timeout_seconds <= 0` disables the
    /// check.
    pub fn detect_stale_running(&self, stale_timeout_seconds: i64) -> Result<Vec<String>> {
        if stale_timeout_seconds <= 0 {
            return Ok(Vec::new());
        }
        let now = Self::now();
        let rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT t.id, t.worker_pid, t.last_heartbeat_at, \
                            COALESCE(r.started_at, t.started_at) \
                       FROM tasks t \
                       LEFT JOIN task_runs r ON r.id = t.current_run_id \
                      WHERE t.status = 'running'",
                )
                .map_err(db_error("stale"))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                })
                .map_err(db_error("stale"))?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error("stale"))?
        };
        let mut reclaimed = Vec::new();
        for (id, worker_pid, last_heartbeat_at, active_started_at) in rows {
            let Some(started) = active_started_at else {
                continue;
            };
            let elapsed = now - started;
            if elapsed < stale_timeout_seconds {
                continue;
            }
            let heartbeat_age = last_heartbeat_at.map(|hb| now - hb);
            if let Some(age) = heartbeat_age {
                if age < 3600 {
                    continue; // recent heartbeat → still alive
                }
            }
            if let Some(pid) = worker_pid {
                if pid_alive(pid) {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                    for _ in 0..10 {
                        if !pid_alive(pid) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                    if pid_alive(pid) {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        if pid_alive(pid) {
                            // Never release a claim while the worker is
                            // still alive — retry next tick (hermes).
                            continue;
                        }
                    }
                }
            }
            let updated = self
                .conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE tasks SET status = 'ready', claim_lock = NULL, \
                     claim_expires = NULL, worker_pid = NULL, last_heartbeat_at = NULL \
                     WHERE id = ?1 AND status = 'running'",
                    params![id],
                )
                .map_err(db_error("stale release"))?;
            if updated != 1 {
                continue;
            }
            let error = match heartbeat_age {
                Some(age) => format!("no heartbeat for {age}s after {elapsed}s running"),
                None => format!("no heartbeat ever after {elapsed}s running"),
            };
            self.close_active_run(&id, "stale", "stale", None, Some(&error))?;
            self.append_event(
                &id,
                "stale",
                serde_json::json!({
                    "elapsed_seconds": elapsed,
                    "last_heartbeat_at": last_heartbeat_at,
                    "heartbeat_age_seconds": heartbeat_age,
                    "timeout_seconds": stale_timeout_seconds,
                    "pid": worker_pid,
                }),
            )?;
            reclaimed.push(id);
        }
        Ok(reclaimed)
    }

    pub fn release_stale_claims(&self) -> Result<Vec<String>> {
        let now = Self::now();
        let stale: Vec<(String, Option<i64>)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, worker_pid FROM tasks WHERE status = 'running' \
                     AND claim_expires IS NOT NULL AND claim_expires < ?1",
                )
                .map_err(db_error("stale"))?;
            let rows = stmt
                .query_map(params![now], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
                })
                .map_err(db_error("stale"))?;
            let found = rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error("stale"))?;
            found
        };
        let mut reclaimed = Vec::new();
        for (id, worker_pid) in stale {
            if let Some(pid) = worker_pid {
                if pid_alive(pid) {
                    // Healthy slow worker: extend the claim another TTL.
                    let new_expires = now + DEFAULT_CLAIM_TTL_SECS;
                    self.conn
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE tasks SET claim_expires = ?2 WHERE id = ?1 \
                             AND status = 'running'",
                            params![id, new_expires],
                        )
                        .map_err(db_error("extend claim"))?;
                    self.append_event(
                        &id,
                        "claim_extended",
                        serde_json::json!({ "expires": new_expires }),
                    )?;
                    continue;
                }
            }
            self.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE tasks SET status = 'ready', claim_lock = NULL, \
                     claim_expires = NULL, worker_pid = NULL \
                     WHERE id = ?1 AND status = 'running'",
                    params![id],
                )
                .map_err(db_error("release claim"))?;
            self.append_event(&id, "released", serde_json::json!({ "reason": "stale_claim" }))?;
            self.close_active_run(&id, "reclaimed", "reclaimed", None, None)?;
            reclaimed.push(id);
        }
        Ok(reclaimed)
    }

    /// Promote `todo` tasks whose parents are all `done` to `ready`
    /// (hermes `recompute_ready`). Returns promoted task ids.
    pub fn recompute_ready(&self) -> Result<Vec<String>> {
        let todos: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id FROM tasks WHERE status = 'todo'")
                .map_err(db_error("recompute"))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(db_error("recompute"))?;
            let found = rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error("recompute"))?;
            found
        };
        let mut promoted = Vec::new();
        for id in todos {
            let parents = self.parents_of(&id)?;
            let mut all_done = true;
            for parent in parents {
                match self.get_task(&parent)? {
                    Some(task) if task.status == "done" => {}
                    _ => {
                        all_done = false;
                        break;
                    }
                }
            }
            if all_done {
                self.ready_task(&id)?;
                promoted.push(id);
            }
        }
        Ok(promoted)
    }

    /// Record a non-success outcome and maybe trip the circuit breaker
    /// (hermes `_record_task_failure`, timeout/crash bookkeeping mode:
    /// the task is already back at `ready` with the claim cleared).
    /// Returns true when the task was auto-blocked. Threshold
    /// resolution: per-task `max_retries` wins over `failure_limit`
    /// (hermes: per-task override > dispatcher config > default 2).
    pub fn record_task_failure(
        &self,
        task_id: &str,
        error: &str,
        outcome: &str,
        failure_limit: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(i64, String, Option<i64>)> = conn
            .query_row(
                "SELECT consecutive_failures, status, max_retries FROM tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error("record failure"))?;
        let Some((consecutive, status, max_retries)) = row else {
            return Ok(false);
        };
        let failures = consecutive + 1;
        let (effective_limit, limit_source) = match max_retries {
            Some(n) => (n, "task"),
            None => (failure_limit, "dispatcher"),
        };
        let err_capped: String = error.chars().take(500).collect();
        if failures >= effective_limit && (status == "ready" || status == "running") {
            // Trip the breaker: ready/running → blocked + gave_up event.
            conn.execute(
                "UPDATE tasks SET status = 'blocked', consecutive_failures = ?2, \
                 last_failure_error = ?3 WHERE id = ?1 AND status IN ('ready', 'running')",
                params![task_id, failures, err_capped],
            )
            .map_err(db_error("record failure trip"))?;
            drop(conn);
            self.append_event(
                task_id,
                "gave_up",
                serde_json::json!({
                    "failures": failures,
                    "effective_limit": effective_limit,
                    "limit_source": limit_source,
                    "error": err_capped,
                    "trigger_outcome": outcome,
                }),
            )?;
            Ok(true)
        } else {
            conn.execute(
                "UPDATE tasks SET consecutive_failures = ?2, last_failure_error = ?3 \
                 WHERE id = ?1",
                params![task_id, failures, err_capped],
            )
            .map_err(db_error("record failure count"))?;
            Ok(false)
        }
    }

    /// Record the pid of a dispatcher-spawned worker.
    pub fn set_worker_pid(&self, id: &str, pid: Option<i64>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET worker_pid = ?2 WHERE id = ?1",
            params![id, pid],
        )
        .map_err(db_error("worker_pid"))?;
        if pid.is_some() {
            // hermes _record_spawned also stamps the active run.
            conn.execute(
                "UPDATE task_runs SET worker_pid = ?2 \
                  WHERE id = (SELECT current_run_id FROM tasks WHERE id = ?1) \
                  AND ended_at IS NULL",
                params![id, pid],
            )
            .map_err(db_error("worker_pid run"))?;
        }
        Ok(())
    }

    /// Run one dispatcher tick (hermes `dispatch_once`, scoped port):
    /// 1. reclaim stale claims, 2. promote parent-done todos, 3. spawn
    /// ready tasks (priority desc, oldest first) up to the live
    /// concurrency cap `max_spawn` (counting already-running tasks).
    /// Before each spawn the task workspace is resolved (hermes
    /// `resolve_workspace`) and persisted; resolution errors are
    /// counted as spawn failures prefixed `workspace:`. `spawn` gets
    /// the resolved workspace and returns the worker pid; failures are
    /// counted per task and after `failure_limit` consecutive failures
    /// the task is auto-blocked with the last error (hermes
    /// DEFAULT_FAILURE_LIMIT = 2). With `use_worktrees`, scratch tasks
    /// created without an explicit `--workspace` keep the pre-P139
    /// behaviour and run in a git worktree.
    pub fn dispatch_once<F>(
        &self,
        home: &Path,
        use_worktrees: bool,
        mut spawn: F,
        max_spawn: Option<usize>,
        dry_run: bool,
        failure_limit: usize,
        stale_timeout_seconds: i64,
    ) -> Result<DispatchResult>
    where
        F: FnMut(&Task, Option<&Path>) -> std::result::Result<Option<i64>, String>,
    {
        let mut result = DispatchResult::default();
        result.reaped = self.reap_timed_out()?;
        result.reclaimed = self.release_stale_claims()?;
        result.stale = self.detect_stale_running(stale_timeout_seconds)?;
        result.crashed = self.detect_crashed_workers()?;
        result.promoted = self.recompute_ready()?;

        let running = self.list_tasks(None, Some("running"), None, 10_000)?;
        let mut running_count = running.len();
        let ready: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM tasks WHERE status = 'ready' AND claim_lock IS NULL \
                     ORDER BY priority DESC, created_at ASC",
                )
                .map_err(db_error("dispatch"))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(db_error("dispatch"))?;
            let found = rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error("dispatch"))?;
            found
        };

        for id in ready {
            if let Some(cap) = max_spawn {
                if running_count >= cap {
                    result.skipped_capped.push(id);
                    continue;
                }
            }
            let Some(task) = self.get_task(&id)? else {
                continue;
            };
            if dry_run {
                result.would_spawn.push(id);
                continue;
            }
            // [kanban] worktrees=true keeps its pre-P139 meaning for
            // tasks created without an explicit --workspace: scratch
            // upgrades to a worktree anchored at the board workdir (or
            // the dispatcher CWD fallback).
            let upgraded;
            let effective: &Task = if use_worktrees
                && task.workspace_kind == "scratch"
                && task
                    .workspace_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .is_none()
            {
                upgraded = Task {
                    workspace_kind: "worktree".to_string(),
                    ..task.clone()
                };
                &upgraded
            } else {
                &task
            };
            // Resolve the workspace BEFORE spawn (hermes dispatch); the
            // resolved path is persisted so retries reuse it.
            let spawn_outcome = match self.resolve_workspace(home, effective) {
                Ok((workspace, branch)) => {
                    let _ = self.set_workspace_path(&id, &workspace);
                    if let Some(branch) = &branch {
                        let _ = self.set_branch_name(&id, branch);
                    }
                    spawn(effective, Some(workspace.as_path()))
                }
                Err(err) => Err(format!("workspace: {err}")),
            };
            match spawn_outcome {
                Ok(pid) => {
                    let claimed = self.claim_task(
                        &id,
                        &KanbanStore::claimer_id(),
                        DEFAULT_CLAIM_TTL_SECS,
                    )?;
                    self.set_worker_pid(&id, pid)?;
                    self.append_event(
                        &id,
                        "spawned",
                        serde_json::json!({ "pid": pid, "assignee": claimed.assignee }),
                    )?;
                    running_count += 1;
                    result.spawned.push((id, pid));
                }
                Err(err) => {
                    self.append_event(
                        &id,
                        "spawn_failed",
                        serde_json::json!({ "error": err }),
                    )?;
                    self.synthesize_closed_run(&id, "spawn_failed", None, Some(&err))?;
                    let gave_up = self.record_task_failure(
                        &id,
                        &err,
                        "spawn_failed",
                        failure_limit as i64,
                    )?;
                    if gave_up {
                        result.auto_blocked.push(id);
                    } else {
                        result.spawn_failed.push(id);
                    }
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, KanbanStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = KanbanStore::open(dir.path().join("kanban.db")).unwrap();
        (dir, store)
    }

    fn make_task(store: &KanbanStore, title: &str) -> Task {
        store
            .create_task(&NewTask {
                title: title.into(),
                created_by: "tester".into(),
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn open_seeds_default_board() {
        let (_dir, store) = temp_store();
        assert_eq!(store.current_board().unwrap(), "default");
        let boards = store.list_boards().unwrap();
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0].slug, "default");
    }

    #[test]
    fn create_list_get_roundtrip() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "Fix the parser");
        assert!(task.id.starts_with("t_"));
        assert_eq!(task.status, "todo");
        assert_eq!(task.board, "default");

        let got = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(got.title, "Fix the parser");

        let listed = store.list_tasks(None, None, None, 100).unwrap();
        assert_eq!(listed.len(), 1);

        // Unique prefix resolution.
        let prefix = &task.id[..5];
        assert_eq!(
            store.resolve_task_id(prefix).unwrap().as_deref(),
            Some(task.id.as_str())
        );
        assert_eq!(store.resolve_task_id("t_nope").unwrap(), None);
    }

    #[test]
    fn lifecycle_ready_claim_complete() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "Ship it");
        store.ready_task(&task.id).unwrap();
        let claimed = store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        assert_eq!(claimed.status, "running");
        assert_eq!(claimed.assignee.as_deref(), Some("host:1"));
        assert!(claimed.started_at.is_some());
        store.heartbeat_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        let done = store.complete_task(&task.id, Some("all green")).unwrap();
        assert_eq!(done.status, "done");
        assert_eq!(done.result.as_deref(), Some("all green"));
        assert!(done.claim_lock.is_none());

        let events = store.events(&task.id).unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["created", "ready", "claimed", "completed"]);
    }

    #[test]
    fn double_claim_fails_but_stale_takeover_works() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "Contended");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "worker-a", 60).unwrap();
        let err = store.claim_task(&task.id, "worker-b", 60).unwrap_err();
        assert!(err.to_string().contains("already claimed"), "{err}");

        // Force the claim into the past; worker-b may take over.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE tasks SET claim_expires = 1 WHERE id = ?1",
                params![task.id],
            )
            .unwrap();
        }
        let taken = store.claim_task(&task.id, "worker-b", 60).unwrap();
        assert_eq!(taken.assignee.as_deref(), Some("worker-a")); // preserved
        assert!(taken.claim_lock.is_some());
    }

    #[test]
    fn block_unblock_and_invalid_transitions() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "Flaky");
        // todo cannot be claimed directly.
        assert!(store.claim_task(&task.id, "w", 60).is_err());
        let blocked = store.block_task(&task.id, "waiting on upstream").unwrap();
        assert_eq!(blocked.status, "blocked");
        // blocked -> ready via unblock.
        let ready = store.unblock_task(&task.id).unwrap();
        assert_eq!(ready.status, "ready");
        // ready -> ready is invalid.
        assert!(store.ready_task(&task.id).is_err());
        // done is terminal for assign.
        store.complete_task(&task.id, None).unwrap();
        assert!(store.assign_task(&task.id, "late").is_err());
    }

    #[test]
    fn comments_and_assignee_filter() {
        let (_dir, store) = temp_store();
        let a = make_task(&store, "A");
        let b = make_task(&store, "B");
        store.assign_task(&a.id, "alice").unwrap();
        store.add_comment(&a.id, "alice", "starting now").unwrap();
        store.add_comment(&a.id, "bob", "gl hf").unwrap();
        let comments = store.comments(&a.id).unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].author, "alice");

        let alice_tasks = store.list_tasks(None, None, Some("alice"), 100).unwrap();
        assert_eq!(alice_tasks.len(), 1);
        assert_eq!(alice_tasks[0].id, a.id);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn boards_crud_and_scoping() {
        let (_dir, store) = temp_store();
        store.create_board("ops", Some("Operations"), None).unwrap();
        assert!(store.create_board("ops", None, None).is_err());
        store.switch_board("ops").unwrap();
        assert_eq!(store.current_board().unwrap(), "ops");
        let task = make_task(&store, "On the ops board");
        assert_eq!(task.board, "ops");
        // default board listing does not see the ops task.
        let default_tasks = store.list_tasks(Some("default"), None, None, 100).unwrap();
        assert!(default_tasks.is_empty());
        // Cannot remove a board with active tasks.
        assert!(store.remove_board("ops").is_err());
        store.archive_task(&task.id).unwrap();
        store.remove_board("ops").unwrap();
        assert_eq!(store.current_board().unwrap(), "default");
        assert!(store.remove_board("default").is_err());
    }

    #[test]
    fn status_icons_cover_hermes_set() {
        for status in STATUSES {
            assert_ne!(status_icon(status), "?");
        }
        assert_eq!(status_icon("weird"), "?");
    }

    #[test]
    fn dispatch_promotes_todos_with_done_parents() {
        let (_dir, store) = temp_store();
        let parent = make_task(&store, "parent");
        let child = make_task(&store, "child");
        store.link_tasks(&parent.id, &child.id).unwrap();
        // Parentless todos promote (vacuous all-parents-done); the child
        // stays todo while its parent is unfinished.
        let promoted = store.recompute_ready().unwrap();
        assert_eq!(promoted, vec![parent.id.clone()]);
        assert_eq!(store.get_task(&child.id).unwrap().unwrap().status, "todo");
        store.complete_task(&parent.id, Some("done")).unwrap();
        let promoted = store.recompute_ready().unwrap();
        assert_eq!(promoted, vec![child.id.clone()]);
        assert_eq!(store.get_task(&child.id).unwrap().unwrap().status, "ready");
    }

    #[test]
    fn dispatch_spawns_ready_tasks_respecting_cap() {
        let (dir, store) = temp_store();
        let first = make_task(&store, "first");
        let second = make_task(&store, "second");
        store.ready_task(&first.id).unwrap();
        store.ready_task(&second.id).unwrap();

        let result = store
            .dispatch_once(dir.path(), false, |_, _| Ok(Some(1234)), Some(1), false, 2, 0)
            .unwrap();
        assert_eq!(result.spawned.len(), 1);
        assert_eq!(result.spawned[0].0, first.id);
        assert_eq!(result.skipped_capped, vec![second.id.clone()]);
        let spawned_task = store.get_task(&first.id).unwrap().unwrap();
        assert_eq!(spawned_task.status, "running");
        assert_eq!(spawned_task.worker_pid, Some(1234));

        // Second tick with a higher cap picks up the remaining task.
        let result = store
            .dispatch_once(dir.path(), false, |_, _| Ok(Some(5678)), Some(2), false, 2, 0)
            .unwrap();
        assert_eq!(result.spawned.len(), 1);
        assert_eq!(result.spawned[0].0, second.id);
    }

    #[test]
    fn dispatch_dry_run_spawns_nothing() {
        let (dir, store) = temp_store();
        let task = make_task(&store, "probe");
        store.ready_task(&task.id).unwrap();
        let result = store
            .dispatch_once(dir.path(), false, |_, _| panic!("dry run must not spawn"), None, true, 2, 0)
            .unwrap();
        assert_eq!(result.would_spawn, vec![task.id.clone()]);
        assert!(result.spawned.is_empty());
        assert_eq!(store.get_task(&task.id).unwrap().unwrap().status, "ready");
    }

    #[test]
    fn dispatch_auto_blocks_after_repeated_spawn_failures() {
        let (dir, store) = temp_store();
        let task = make_task(&store, "doomed");
        store.ready_task(&task.id).unwrap();

        // First failure: recorded, still ready-ish for retry.
        let result = store
            .dispatch_once(dir.path(), false, |_, _| Err("boom".into()), None, false, 2, 0)
            .unwrap();
        assert_eq!(result.spawn_failed, vec![task.id.clone()]);
        assert!(result.auto_blocked.is_empty());

        // Second consecutive failure trips the limit → blocked.
        let result = store
            .dispatch_once(dir.path(), false, |_, _| Err("boom again".into()), None, false, 2, 0)
            .unwrap();
        assert_eq!(result.auto_blocked, vec![task.id.clone()]);
        let blocked = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(blocked.status, "blocked");
        assert!(blocked.result.is_none());
    }

    #[test]
    fn release_stale_claims_reclaims_dead_workers() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "stale");
        store.ready_task(&task.id).unwrap();
        let claimed = store
            .claim_task(&task.id, &KanbanStore::claimer_id(), DEFAULT_CLAIM_TTL_SECS)
            .unwrap();
        assert_eq!(claimed.status, "running");

        // Force-expire the claim and pin a dead worker pid.
        store.conn.lock().unwrap()
            .execute(
                "UPDATE tasks SET claim_expires = ?2, worker_pid = ?3 WHERE id = ?1",
                params![task.id, 1i64, 999_999_999i64],
            )
            .unwrap();
        let reclaimed = store.release_stale_claims().unwrap();
        assert_eq!(reclaimed, vec![task.id.clone()]);
        let released = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(released.status, "ready");
        assert!(released.claim_lock.is_none());
        assert!(released.worker_pid.is_none());
    }

    #[test]
    fn swarm_graph_topology_and_flow() {
        let (_dir, store) = temp_store();
        let workers = vec![
            SwarmWorkerSpec {
                assignee: "alice".into(),
                title: "Research".into(),
                body: "find facts".into(),
                priority: 0,
                skills: vec![],
            },
            SwarmWorkerSpec {
                assignee: "bob".into(),
                title: "Draft".into(),
                body: "write draft".into(),
                priority: 0,
                skills: vec![],
            },
        ];
        let created = store
            .create_swarm("Ship a report", &workers, "carol", "dave", "", None)
            .unwrap();
        assert_eq!(created.worker_ids.len(), 2);

        // Root: done, blackboard carries the topology.
        let root = store.get_task(&created.root_id).unwrap().unwrap();
        assert_eq!(root.status, "done");
        let blackboard = store.latest_blackboard(&created.root_id).unwrap().unwrap();
        let topology = blackboard.get("topology").unwrap();
        assert_eq!(topology["verifier_id"], created.verifier_id);

        // Workers: ready with swarm context in the body.
        for id in &created.worker_ids {
            let task = store.get_task(id).unwrap().unwrap();
            assert_eq!(task.status, "ready");
            assert!(task.body.contains("## Swarm protocol"));
            assert_eq!(store.parents_of(id).unwrap(), vec![created.root_id.clone()]);
        }

        // Verifier waits for BOTH workers; synthesizer waits for verifier.
        let verifier_parents = store.parents_of(&created.verifier_id).unwrap();
        assert_eq!(verifier_parents.len(), 2);
        assert_eq!(
            store.parents_of(&created.synthesizer_id).unwrap(),
            vec![created.verifier_id.clone()]
        );

        // Flow: complete workers -> verifier promotes; complete verifier ->
        // synthesizer promotes.
        assert!(store.recompute_ready().unwrap().is_empty());
        for id in &created.worker_ids {
            store.complete_task(id, Some("ok")).unwrap();
        }
        let promoted = store.recompute_ready().unwrap();
        assert_eq!(promoted, vec![created.verifier_id.clone()]);
        store.complete_task(&created.verifier_id, Some("verified")).unwrap();
        let promoted = store.recompute_ready().unwrap();
        assert_eq!(promoted, vec![created.synthesizer_id.clone()]);
    }

    #[test]
    fn worktree_lifecycle_prepare_reuse_gc() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "init"]);

        let store = KanbanStore::open(repo.join("kanban.db")).unwrap();
        let task = make_task(&store, "worktree task");

        // Prepare creates the worktree; a second call reuses it.
        let wt = prepare_worktree(repo, &task).unwrap().expect("in a git repo");
        assert!(wt.ends_with(format!(".worktrees/{}", task.id)));
        assert!(wt.is_dir());
        let wt2 = prepare_worktree(repo, &task).unwrap();
        assert_eq!(Some(wt.clone()), wt2);

        // Active task: gc keeps the tree.
        let (removed, kept) = gc_worktrees(repo, &store).unwrap();
        assert_eq!((removed, kept), (0, 1));

        // Done task: gc removes the tree (branch stays).
        store.complete_task(&task.id, Some("done")).unwrap();
        let (removed, kept) = gc_worktrees(repo, &store).unwrap();
        assert_eq!((removed, kept), (1, 0));
        assert!(!wt.is_dir());
    }

    #[test]
    fn release_stale_claims_extends_live_workers() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "slow-but-alive");
        store.ready_task(&task.id).unwrap();
        store
            .claim_task(&task.id, &KanbanStore::claimer_id(), DEFAULT_CLAIM_TTL_SECS)
            .unwrap();
        // Expired claim but our own (very much alive) pid.
        let live_pid = std::process::id() as i64;
        store.conn.lock().unwrap()
            .execute(
                "UPDATE tasks SET claim_expires = ?2, worker_pid = ?3 WHERE id = ?1",
                params![task.id, 1i64, live_pid],
            )
            .unwrap();
        let reclaimed = store.release_stale_claims().unwrap();
        assert!(reclaimed.is_empty());
        let extended = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(extended.status, "running");
        assert!(extended.claim_expires.unwrap() > 1);
    }

    #[test]
    fn create_task_idempotency_returns_existing() {
        let (_dir, store) = temp_store();
        let first = store
            .create_task(&NewTask {
                title: "deploy".into(),
                created_by: "tester".into(),
                idempotency_key: Some("deploy-v1".into()),
                ..Default::default()
            })
            .unwrap();
        let second = store
            .create_task(&NewTask {
                title: "deploy (again)".into(),
                created_by: "tester".into(),
                idempotency_key: Some("deploy-v1".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.title, "deploy"); // original task, not the duplicate
        let all = store.list_tasks(None, None, None, 100).unwrap();
        assert_eq!(all.len(), 1);
        // A different key still creates a fresh task.
        let third = store
            .create_task(&NewTask {
                title: "deploy v2".into(),
                created_by: "tester".into(),
                idempotency_key: Some("deploy-v2".into()),
                ..Default::default()
            })
            .unwrap();
        assert_ne!(third.id, first.id);
    }

    fn swarm_specs() -> Vec<SwarmWorkerSpec> {
        vec![
            SwarmWorkerSpec {
                assignee: "alice".into(),
                title: "Research".into(),
                body: "find facts".into(),
                priority: 0,
                skills: vec!["deep-research".into()],
            },
            SwarmWorkerSpec {
                assignee: "bob".into(),
                title: "Draft".into(),
                body: "write draft".into(),
                priority: 0,
                skills: vec![],
            },
        ]
    }

    #[test]
    fn swarm_skills_are_recorded() {
        let (_dir, store) = temp_store();
        let created = store
            .create_swarm("Ship a report", &swarm_specs(), "carol", "dave", "", None)
            .unwrap();
        let researcher = store.get_task(&created.worker_ids[0]).unwrap().unwrap();
        assert_eq!(researcher.skills.as_deref(), Some(&["deep-research".to_string()][..]));
        let drafter = store.get_task(&created.worker_ids[1]).unwrap().unwrap();
        assert!(drafter.skills.is_none());
        let verifier = store.get_task(&created.verifier_id).unwrap().unwrap();
        assert_eq!(
            verifier.skills.as_deref(),
            Some(&["requesting-code-review".to_string()][..])
        );
        let synthesizer = store.get_task(&created.synthesizer_id).unwrap().unwrap();
        assert_eq!(synthesizer.skills.as_deref(), Some(&["humanizer".to_string()][..]));
    }

    #[test]
    fn swarm_idempotency_key_recovers_topology() {
        let (_dir, store) = temp_store();
        let first = store
            .create_swarm("Ship a report", &swarm_specs(), "carol", "dave", "", Some("swarm-42"))
            .unwrap();
        let before = store.list_tasks(None, None, None, 500).unwrap().len();
        let second = store
            .create_swarm("Ship a report", &swarm_specs(), "carol", "dave", "", Some("swarm-42"))
            .unwrap();
        assert_eq!(first.root_id, second.root_id);
        assert_eq!(first.worker_ids, second.worker_ids);
        assert_eq!(first.verifier_id, second.verifier_id);
        assert_eq!(first.synthesizer_id, second.synthesizer_id);
        let after = store.list_tasks(None, None, None, 500).unwrap().len();
        assert_eq!(before, after, "no duplicate graph may be created");
    }

    #[test]
    fn reap_timed_out_requeues_expired_worker() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "long job".into(),
                created_by: "tester".into(),
                max_runtime_seconds: Some(5),
                ..Default::default()
            })
            .unwrap();
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:test", 60).unwrap();
        // Age the attempt past the cap.
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE tasks SET started_at = started_at - 60 WHERE id = ?1",
                rusqlite::params![task.id],
            )
            .unwrap();

        // Under the cap → untouched; over the cap → reaped back to ready.
        assert!(store.reap_timed_out().unwrap().is_empty() == false);
        let reaped = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(reaped.status, "ready");
        assert!(reaped.claim_lock.is_none());
        assert!(reaped.worker_pid.is_none());
        let events = store.events(&task.id).unwrap();
        assert!(events.iter().any(|e| e.kind == "timed_out"));
    }

    #[test]
    fn worker_prompt_force_loads_skills() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let skill_dir = home.join("skills").join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Test Skill\nAlways sign off with DONE-SKILL.").unwrap();
        let task = Task {
            id: "t_1".into(),
            board: "default".into(),
            title: "job".into(),
            body: String::new(),
            assignee: None,
            status: "ready".into(),
            priority: 0,
            created_by: "tester".into(),
            created_at: 0,
            started_at: None,
            completed_at: None,
            tenant: None,
            model: None,
            result: None,
            claim_lock: None,
            claim_expires: None,
            last_heartbeat_at: None,
            worker_pid: None,
            skills: Some(vec!["test-skill".into()]),
            max_runtime_seconds: None,
            idempotency_key: None,
            consecutive_failures: 0,
            last_failure_error: None,
            max_retries: None,
        
            workspace_kind: "scratch".into(),
            workspace_path: None,
            branch_name: None,};
        let prompt = worker_prompt(home, &task);
        assert!(prompt.contains("kanban worker for task t_1"));
        assert!(prompt.contains("DONE-SKILL"), "skill body must be inlined");
    }

    #[test]
    fn create_task_triage_parks_in_triage_column() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "rough idea".into(),
                created_by: "tester".into(),
                triage: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(task.status, "triage");
        assert_eq!(status_icon("triage"), "\u{1FA7A}");
    }

    #[test]
    fn specify_triage_task_promotes_and_updates() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "rough idea".into(),
                body: "one liner".into(),
                created_by: "tester".into(),
                triage: true,
                ..Default::default()
            })
            .unwrap();
        let ok = store
            .specify_triage_task(
                &task.id,
                Some("Build the widget"),
                Some("**Goal** — ship it"),
                Some("alice"),
                "specifier",
            )
            .unwrap();
        assert!(ok);
        let specified = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(specified.status, "todo");
        assert_eq!(specified.title, "Build the widget");
        assert_eq!(specified.body, "**Goal** — ship it");
        assert_eq!(specified.assignee.as_deref(), Some("alice"));
        // A task already out of triage is not re-specified.
        let again = store
            .specify_triage_task(&task.id, Some("X"), None, None, "specifier")
            .unwrap();
        assert!(!again);
    }

    fn triage_root(store: &KanbanStore) -> Task {
        store
            .create_task(&NewTask {
                title: "Ship the report pipeline".into(),
                body: "big vague idea".into(),
                created_by: "tester".into(),
                triage: true,
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn decompose_triage_task_fans_out_and_wakes_root() {
        let (_dir, store) = temp_store();
        let root = triage_root(&store);
        let children = vec![
            DecomposeChild {
                title: "Research".into(),
                body: "find facts".into(),
                assignee: Some("alice".into()),
                parents: vec![],
            },
            DecomposeChild {
                title: "Draft".into(),
                body: "write draft".into(),
                assignee: Some("bob".into()),
                parents: vec![0],
            },
        ];
        let child_ids = store
            .decompose_triage_task(&root.id, Some("carol"), &children, "tester", true)
            .unwrap()
            .expect("decomposition succeeds");
        assert_eq!(child_ids.len(), 2);

        // Root flipped to todo under the orchestrator, children created.
        let root_after = store.get_task(&root.id).unwrap().unwrap();
        assert_eq!(root_after.status, "todo");
        assert_eq!(root_after.assignee.as_deref(), Some("carol"));
        let research = store.get_task(&child_ids[0]).unwrap().unwrap();
        let draft = store.get_task(&child_ids[1]).unwrap().unwrap();
        assert_eq!(research.status, "ready"); // parent-free, auto-promoted
        assert_eq!(draft.status, "todo"); // waits on research

        // Root waits for the whole graph (child-of-every-child).
        let root_parents = store.parents_of(&root.id).unwrap();
        assert_eq!(root_parents.len(), 2);

        // Complete the graph: root wakes up to ready.
        store.ready_task(&research.id).ok();
        store.claim_task(&research.id, "host:test", 60).unwrap();
        store.complete_task(&research.id, Some("facts")).unwrap();
        store.recompute_ready().unwrap();
        let draft = store.get_task(&child_ids[1]).unwrap().unwrap();
        assert_eq!(draft.status, "ready");
        store.claim_task(&draft.id, "host:test", 60).unwrap();
        store.complete_task(&draft.id, Some("draft")).unwrap();
        store.recompute_ready().unwrap();
        let root_final = store.get_task(&root.id).unwrap().unwrap();
        assert_eq!(root_final.status, "ready");
    }

    #[test]
    fn decompose_rejects_cycles_and_non_triage_roots() {
        let (_dir, store) = temp_store();
        let root = triage_root(&store);
        let cyclic = vec![
            DecomposeChild {
                title: "A".into(),
                body: String::new(),
                assignee: None,
                parents: vec![1],
            },
            DecomposeChild {
                title: "B".into(),
                body: String::new(),
                assignee: None,
                parents: vec![0],
            },
        ];
        let err = store
            .decompose_triage_task(&root.id, None, &cyclic, "tester", true)
            .unwrap_err();
        assert!(err.to_string().contains("cyclic"));

        let plain = make_task(&store, "not triage");
        let children = vec![DecomposeChild {
            title: "A".into(),
            body: String::new(),
            assignee: None,
            parents: vec![],
        }];
        let outcome = store
            .decompose_triage_task(&plain.id, None, &children, "tester", true)
            .unwrap();
        assert!(outcome.is_none());
    }

    #[test]
    fn schedule_unblock_roundtrip() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "wait for friday");
        let scheduled = store.schedule_task(&task.id, "until the release train").unwrap();
        assert_eq!(scheduled.status, "scheduled");
        let comments = store.comments(&task.id).unwrap();
        assert!(comments.iter().any(|c| c.body.contains("release train")));
        let unblocked = store.unblock_task(&task.id).unwrap();
        assert_eq!(unblocked.status, "ready");
    }

    #[test]
    fn promote_respects_parents_unless_forced() {
        let (_dir, store) = temp_store();
        let parent = make_task(&store, "parent");
        let child = make_task(&store, "child");
        store.link_tasks(&parent.id, &child.id).unwrap();
        let err = store.promote_task(&child.id, "", false).unwrap_err();
        assert!(err.to_string().contains("--force"));
        let forced = store.promote_task(&child.id, "override", true).unwrap();
        assert_eq!(forced.status, "ready");
        // Parent-done children promote without force.
        let child2 = make_task(&store, "child2");
        store.link_tasks(&parent.id, &child2.id).unwrap();
        store.ready_task(&parent.id).unwrap();
        store.claim_task(&parent.id, "host:test", 60).unwrap();
        store.complete_task(&parent.id, None).unwrap();
        let promoted = store.promote_task(&child2.id, "", false).unwrap();
        assert_eq!(promoted.status, "ready");
    }

    #[test]
    fn reclaim_releases_running_claim() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "busy");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:test", 60).unwrap();
        store.set_worker_pid(&task.id, Some(999_999)).unwrap();
        let reclaimed = store.reclaim_task(&task.id, "stuck worker").unwrap();
        assert_eq!(reclaimed.status, "ready");
        assert!(reclaimed.claim_lock.is_none());
        assert!(reclaimed.worker_pid.is_none());
    }

    #[test]
    fn reassign_with_reclaim_on_running_task() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "running one");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:test", 60).unwrap();
        let reassigned = store
            .reassign_task(&task.id, "other-profile", true, "handoff")
            .unwrap();
        assert_eq!(reassigned.status, "ready");
        assert_eq!(reassigned.assignee.as_deref(), Some("other-profile"));
        let cleared = store.reassign_task(&task.id, "none", false, "").unwrap();
        assert!(cleared.assignee.is_none());
    }

    #[test]
    fn edit_title_and_body() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "old title");
        let edited = store
            .edit_task(&task.id, Some("new title"), Some("new body"))
            .unwrap();
        assert_eq!(edited.title, "new title");
        assert_eq!(edited.body, "new body");
        let body_only = store.edit_task(&task.id, None, Some("body2")).unwrap();
        assert_eq!(body_only.title, "new title");
        assert_eq!(body_only.body, "body2");
        assert!(store.edit_task(&task.id, Some("  "), None).is_err());
    }

    #[test]
    fn set_model_override_and_clear() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "model task");
        let pinned = store.set_model(&task.id, Some("gpt-5.2")).unwrap();
        assert_eq!(pinned.model.as_deref(), Some("gpt-5.2"));
        let cleared = store.set_model(&task.id, None).unwrap();
        assert!(cleared.model.is_none());
    }

    #[test]
    fn attachments_lifecycle_with_removal() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "with files");
        store.attach(&task.id, "file", "/tmp/a.txt").unwrap();
        store.attach(&task.id, "file", "/tmp/b.txt").unwrap();
        let rows = store.attachments_with_ids(&task.id).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(store.remove_attachment(rows[0].0).unwrap());
        assert!(!store.remove_attachment(rows[0].0).unwrap());
        assert_eq!(store.attachments_with_ids(&task.id).unwrap().len(), 1);
    }

    #[test]
    fn board_rename_workdir_and_stats() {
        let (_dir, store) = temp_store();
        store.create_board("ops", Some("Operations"), Some("/srv")).unwrap();
        store.rename_board("ops", "Ops Board").unwrap();
        store.set_board_workdir("ops", Some("/tmp/ops")).unwrap();
        let boards = store.list_boards().unwrap();
        let ops = boards.iter().find(|b| b.slug == "ops").unwrap();
        assert_eq!(ops.name, "Ops Board");

        make_task(&store, "one");
        make_task(&store, "two");
        let counts = store.board_status_counts().unwrap();
        let todo = counts.iter().find(|(s, _)| s == "todo").map(|(_, n)| *n);
        assert_eq!(todo, Some(2));
    }

    #[test]
    fn board_stats_counts_statuses_assignees_and_ready_age() {
        let (_dir, store) = temp_store();
        let a = store
            .create_task(&NewTask {
                title: "a".into(),
                assignee: Some("alice".into()),
                created_by: "tester".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .create_task(&NewTask {
                title: "b".into(),
                assignee: Some("alice".into()),
                created_by: "tester".into(),
                ..Default::default()
            })
            .unwrap();
        store.ready_task(&a.id).unwrap();
        let stats = store.board_stats().unwrap();
        let todo = stats.by_status.iter().find(|(s, _)| s == "todo").map(|(_, n)| *n);
        let ready = stats.by_status.iter().find(|(s, _)| s == "ready").map(|(_, n)| *n);
        assert_eq!(todo, Some(1));
        assert_eq!(ready, Some(1));
        let alice = stats.by_assignee.iter().find(|(name, _)| name == "alice").unwrap();
        assert_eq!(alice.1.iter().map(|(_, n)| *n).sum::<i64>(), 2);
        assert!(stats.oldest_ready_age_seconds.unwrap() >= 0);
    }

    #[test]
    fn board_events_since_filters_and_streams() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "watched".into(),
                assignee: Some("bob".into()),
                created_by: "tester".into(),
                ..Default::default()
            })
            .unwrap();
        let start = store.last_event_id().unwrap();
        store.ready_task(&task.id).unwrap();
        store.block_task(&task.id, "nope").unwrap();
        store.unblock_task(&task.id).unwrap();

        // No filters: everything since the start point, paired with titles.
        let all = store.board_events_since(start, None, None, 100).unwrap();
        assert!(all.len() >= 3);
        assert!(all.iter().all(|(_, title)| title == "watched"));

        // Kind filter narrows the stream.
        let kinds = vec!["blocked".to_string()];
        let blocked_only = store
            .board_events_since(start, None, Some(&kinds), 100)
            .unwrap();
        assert_eq!(blocked_only.len(), 1);
        assert_eq!(blocked_only[0].0.kind, "blocked");

        // Assignee filter: bob matches, carol sees nothing.
        let bob = store.board_events_since(start, Some("bob"), None, 100).unwrap();
        assert!(!bob.is_empty());
        let carol = store.board_events_since(start, Some("carol"), None, 100).unwrap();
        assert!(carol.is_empty());
    }

    #[test]
    fn notify_sub_lifecycle() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "notify me");
        store.ready_task(&task.id).unwrap();

        // Unknown task is rejected (hermes _cmd_notify_subscribe guard).
        let err = store
            .add_notify_sub(
                "t_nope",
                &NewNotifySub {
                    platform: "telegram",
                    chat_id: "1",
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("no such task"));

        store
            .add_notify_sub(
                &task.id,
                &NewNotifySub {
                    platform: "telegram",
                    chat_id: "42",
                    chat_type: Some("group"),
                    user_id: Some("u7"),
                    notifier_profile: Some("default"),
                    ..Default::default()
                },
            )
            .unwrap();

        // Cursor snaps to the current max event id (no history replay).
        let max_event = store.events(&task.id).unwrap().last().unwrap().id;
        let subs = store.list_notify_subs(Some(&task.id)).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].platform, "telegram");
        assert_eq!(subs[0].chat_id, "42");
        assert_eq!(subs[0].chat_type.as_deref(), Some("group"));
        assert_eq!(subs[0].thread_id, "");
        assert_eq!(subs[0].notifier_profile.as_deref(), Some("default"));
        assert_eq!(subs[0].last_event_id, max_event);

        // Duplicate subscribe is idempotent (INSERT OR IGNORE path).
        store
            .add_notify_sub(
                &task.id,
                &NewNotifySub {
                    platform: "telegram",
                    chat_id: "42",
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.list_notify_subs(None).unwrap().len(), 1);

        // Same chat in a thread is a distinct subscription.
        store
            .add_notify_sub(
                &task.id,
                &NewNotifySub {
                    platform: "telegram",
                    chat_id: "42",
                    thread_id: Some("t9"),
                    delivery_metadata: Some(serde_json::json!({"reply_to": 5})),
                    ..Default::default()
                },
            )
            .unwrap();
        let subs = store.list_notify_subs(Some(&task.id)).unwrap();
        assert_eq!(subs.len(), 2);
        let threaded = subs.iter().find(|s| s.thread_id == "t9").unwrap();
        assert_eq!(
            threaded.delivery_metadata.as_ref().unwrap()["reply_to"],
            5
        );

        // Unsubscribe: exact key match only.
        assert!(!store
            .remove_notify_sub(&task.id, "telegram", "42", Some("zz"))
            .unwrap());
        assert!(store
            .remove_notify_sub(&task.id, "telegram", "42", Some("t9"))
            .unwrap());
        assert!(store
            .remove_notify_sub(&task.id, "telegram", "42", None)
            .unwrap());
        assert!(store.list_notify_subs(Some(&task.id)).unwrap().is_empty());
    }

    #[test]
    fn notify_unseen_events_and_cursor() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "watched task");
        store
            .add_notify_sub(
                &task.id,
                &NewNotifySub {
                    platform: "slack",
                    chat_id: "C1",
                    ..Default::default()
                },
            )
            .unwrap();

        store.ready_task(&task.id).unwrap();
        store.append_event(&task.id, "heartbeat", Value::Null).unwrap();
        store.complete_task(&task.id, Some("done!")).unwrap();

        // Everything after the subscribe cursor, kind-filtered or not.
        let (cursor, events) = store
            .unseen_events_for_sub(&task.id, "slack", "C1", None, None)
            .unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["ready", "heartbeat", "completed"]);
        assert_eq!(cursor, events.last().unwrap().id);

        let (_, done_only) = store
            .unseen_events_for_sub(&task.id, "slack", "C1", None, Some(&["completed"]))
            .unwrap();
        assert_eq!(done_only.len(), 1);

        // Unknown sub returns an empty page; cursor advances on request.
        let (cursor2, _) = store
            .unseen_events_for_sub(&task.id, "slack", "nope", None, None)
            .unwrap();
        assert_eq!(cursor2, 0);
        store
            .advance_notify_cursor(&task.id, "slack", "C1", None, cursor)
            .unwrap();
        let (_, events) = store
            .unseen_events_for_sub(&task.id, "slack", "C1", None, None)
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn worker_log_read_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert_eq!(read_worker_log(home, "t_x", None), None);

        let log_dir = home.join("kanban").join("worker-logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(log_dir.join("t_x.log"), b"line one\nline two\nline three\n").unwrap();

        // Full read.
        assert_eq!(
            read_worker_log(home, "t_x", None).unwrap(),
            "line one\nline two\nline three\n"
        );
        // Tail larger than the file returns everything.
        assert_eq!(
            read_worker_log(home, "t_x", Some(1000)).unwrap(),
            "line one\nline two\nline three\n"
        );
        // Tail from mid-line skips the partial first line.
        assert_eq!(
            read_worker_log(home, "t_x", Some(24)).unwrap(),
            "line two\nline three\n"
        );

        // One giant line with no newline: raw tail, nothing skipped.
        std::fs::write(log_dir.join("t_big.log"), "x".repeat(100)).unwrap();
        let tail = read_worker_log(home, "t_big", Some(50)).unwrap();
        assert_eq!(tail, "x".repeat(50));
    }

    #[test]
    fn run_lifecycle_claim_complete() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "attempt history");
        assert!(store.latest_run(&task.id).unwrap().is_none());

        store.ready_task(&task.id).unwrap();
        let claimed = store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert_eq!(run.status, "running");
        assert_eq!(run.profile.as_deref(), Some("host:1"));
        assert!(run.claim_lock.is_some());
        assert!(run.ended_at.is_none());
        assert_eq!(claimed.status, "running");

        store.heartbeat_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert!(run.last_heartbeat_at.is_some());

        store.set_worker_pid(&task.id, Some(4242)).unwrap();
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert_eq!(run.worker_pid, Some(4242));

        store.complete_task(&task.id, Some("shipped")).unwrap();
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert_eq!(run.status, "done");
        assert_eq!(run.outcome.as_deref(), Some("completed"));
        assert_eq!(run.summary.as_deref(), Some("shipped"));
        assert!(run.ended_at.is_some());
        assert!(run.worker_pid.is_none(), "claim machinery cleared on close");
        assert_eq!(
            store.latest_summary(&task.id).unwrap().as_deref(),
            Some("shipped")
        );

        // No active run remains: closing again is a no-op.
        assert_eq!(
            store.close_active_run(&task.id, "done", "completed", None, None).unwrap(),
            None
        );
        // Closed runs survive the include_active=false view.
        assert_eq!(store.list_runs(&task.id, false, None, None).unwrap().len(), 1);
    }

    #[test]
    fn run_block_reclaim_and_retries() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "retry me");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        store.block_task(&task.id, "needs api key").unwrap();
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert_eq!(run.outcome.as_deref(), Some("blocked"));
        assert_eq!(run.error.as_deref(), Some("needs api key"));

        // Retry: unblock → re-claim opens run #2.
        store.unblock_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        store.reclaim_task(&task.id, "manual takeover").unwrap();
        let runs = store.list_runs(&task.id, true, None, None).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].outcome.as_deref(), Some("blocked"));
        assert_eq!(runs[1].outcome.as_deref(), Some("reclaimed"));
        assert_eq!(runs[1].summary.as_deref(), Some("manual takeover"));

        // State filters.
        let blocked = store.list_runs(&task.id, true, Some("outcome"), Some("blocked")).unwrap();
        assert_eq!(blocked.len(), 1);
        let none = store.list_runs(&task.id, true, Some("outcome"), Some("completed")).unwrap();
        assert!(none.is_empty());
        assert!(store.list_runs(&task.id, true, Some("outcome"), None).is_err());
        assert!(store.list_runs(&task.id, true, Some("bogus"), Some("x")).is_err());
    }

    #[test]
    fn run_synthesized_for_unclaimed_complete_and_spawn_failure() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "cli done");
        // CLI-style complete without a claim synthesizes an instant run.
        store.complete_task(&task.id, Some("manual")).unwrap();
        let runs = store.list_runs(&task.id, true, None, None).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].outcome.as_deref(), Some("completed"));
        assert_eq!(runs[0].started_at, runs[0].ended_at.unwrap());

        // Dispatcher spawn failure records an instant spawn_failed run.
        let other = make_task(&store, "bad spawn");
        store.synthesize_closed_run(&other.id, "spawn_failed", None, Some("exec format error")).unwrap();
        let failed = store.list_runs(&other.id, true, Some("outcome"), Some("spawn_failed")).unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].error.as_deref(), Some("exec format error"));
    }

    #[test]
    fn run_reclaim_stale_on_reclaim_by_new_claimer() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "stale run");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        // Force-expire the claim so a second claimer can take over.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE tasks SET claim_expires = 1 WHERE id = ?1",
                params![task.id],
            )
            .unwrap();
        }
        store.claim_task(&task.id, "host:2", DEFAULT_CLAIM_TTL_SECS).unwrap();
        let runs = store.list_runs(&task.id, true, None, None).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].outcome.as_deref(), Some("reclaimed"));
        assert_eq!(runs[1].status, "running");
        // hermes semantics: the run profile is the task's assignee at
        // claim time (set by the first claim), not the new lock holder.
        assert_eq!(runs[1].profile.as_deref(), Some("host:1"));
    }

    #[test]
    fn worker_context_includes_history_and_handoffs() {
        let (_dir, store) = temp_store();
        // Parent completes with a result the child should inherit.
        let parent = make_task(&store, "Design schema");
        store.ready_task(&parent.id).unwrap();
        store.claim_task(&parent.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        store.complete_task(&parent.id, Some("schema v2 shipped")).unwrap();

        let child = make_task(&store, "Implement schema");
        store.link_tasks(&parent.id, &child.id).unwrap();
        store.assign_task(&child.id, "host:1").unwrap();
        store.add_comment(&child.id, "host:1", "starting now").unwrap();

        // Two attempts: first reclaimed, second running.
        store.ready_task(&child.id).unwrap();
        store.claim_task(&child.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        store.reclaim_task(&child.id, "took too long").unwrap();
        store.claim_task(&child.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();

        let ctx = store.build_worker_context(&child.id).unwrap();
        assert!(ctx.contains(&format!("# Kanban task {}: Implement schema", child.id)));
        assert!(ctx.contains("## Prior attempts on this task"));
        assert!(ctx.contains("Attempt 1 — reclaimed"));
        assert!(ctx.contains("took too long"));
        assert!(ctx.contains("## Parent task results"));
        assert!(ctx.contains(&parent.id));
        assert!(ctx.contains("schema v2 shipped"));
        assert!(ctx.contains("## Comment thread"));
        assert!(ctx.contains("starting now"));
        // The active run (attempt 2) must not appear as prior work.
        assert!(!ctx.contains("Attempt 2"));
    }

    #[test]
    fn worker_context_caps_runaway_fields() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "big");
        let big_body = "x".repeat(CTX_MAX_BODY_BYTES + 100);
        store.edit_task(&task.id, None, Some(&big_body)).unwrap();
        let big_comment = "y".repeat(CTX_MAX_COMMENT_BYTES + 50);
        store.add_comment(&task.id, "worker", &big_comment).unwrap();

        let ctx = store.build_worker_context(&task.id).unwrap();
        assert!(ctx.contains("… [truncated,"));
        // No full 8 KB body survived the cap.
        assert!(!ctx.contains(&"x".repeat(CTX_MAX_BODY_BYTES + 1)));
    }

    #[test]
    fn failure_breaker_trips_on_repeated_spawn_failures() {
        let (dir, store) = temp_store();
        let task = make_task(&store, "bad spawn");
        store.ready_task(&task.id).unwrap();

        let tick = || {
            store
                .dispatch_once(dir.path(), false, |_, _| Err("exec format error".to_string()), Some(4), false, 2, 0)
                .unwrap()
        };

        // First failure: counted, task stays ready for retry.
        let r1 = tick();
        assert_eq!(r1.spawn_failed, vec![task.id.clone()]);
        assert!(r1.auto_blocked.is_empty());
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.status, "ready");
        assert_eq!(t.consecutive_failures, 1);
        assert_eq!(t.last_failure_error.as_deref(), Some("exec format error"));

        // Second failure trips the breaker: blocked + gave_up event.
        let r2 = tick();
        assert_eq!(r2.auto_blocked, vec![task.id.clone()]);
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.status, "blocked");
        assert_eq!(t.consecutive_failures, 2);
        let gave_up = store
            .events(&task.id)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "gave_up")
            .unwrap();
        assert_eq!(gave_up.payload["limit_source"], "dispatcher");
        assert_eq!(gave_up.payload["trigger_outcome"], "spawn_failed");
    }

    #[test]
    fn failure_breaker_honors_max_retries_override() {
        let (dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "trip first".into(),
                created_by: "tester".into(),
                max_retries: Some(1),
                ..Default::default()
            })
            .unwrap();
        store.ready_task(&task.id).unwrap();
        let result = store
            .dispatch_once(dir.path(), false, |_, _| Err("boom".to_string()), Some(4), false, 2, 0)
            .unwrap();
        assert_eq!(result.auto_blocked, vec![task.id.clone()]);
        assert!(result.spawn_failed.is_empty());
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.status, "blocked");
        let gave_up = store
            .events(&task.id)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "gave_up")
            .unwrap();
        assert_eq!(gave_up.payload["limit_source"], "task");
        assert_eq!(gave_up.payload["effective_limit"], 1);
    }

    #[test]
    fn failure_counter_resets_on_complete_and_unblock() {
        let (dir, store) = temp_store();
        let task = make_task(&store, "resilient");
        store.ready_task(&task.id).unwrap();
        store
            .dispatch_once(dir.path(), false, |_, _| Err("flaky".to_string()), Some(4), false, 2, 0)
            .unwrap();
        assert_eq!(
            store.get_task(&task.id).unwrap().unwrap().consecutive_failures,
            1
        );

        // Successful run clears the budget.
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        store.complete_task(&task.id, Some("ok")).unwrap();
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.consecutive_failures, 0);
        assert!(t.last_failure_error.is_none());

        // Blocked → unblock also grants a fresh budget (hermes policy).
        let other = make_task(&store, "blocked cycle");
        store.ready_task(&other.id).unwrap();
        store
            .dispatch_once(dir.path(), false, |_, _| Err("flaky".to_string()), Some(4), false, 2, 0)
            .unwrap();
        store
            .dispatch_once(dir.path(), false, |_, _| Err("flaky".to_string()), Some(4), false, 2, 0)
            .unwrap();
        let t = store.get_task(&other.id).unwrap().unwrap();
        assert_eq!(t.status, "blocked");
        store.unblock_task(&other.id).unwrap();
        let t = store.get_task(&other.id).unwrap().unwrap();
        assert_eq!(t.consecutive_failures, 0);
    }

    #[test]
    fn timed_out_consumes_retry_budget() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "slowpoke".into(),
                created_by: "tester".into(),
                max_runtime_seconds: Some(0),
                max_retries: Some(1),
                ..Default::default()
            })
            .unwrap();
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        let reaped = store.reap_timed_out().unwrap();
        assert_eq!(reaped, vec![task.id.clone()]);
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.status, "blocked");
        let gave_up = store
            .events(&task.id)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "gave_up")
            .unwrap();
        assert_eq!(gave_up.payload["trigger_outcome"], "timed_out");
    }

    fn backdate_task(store: &KanbanStore, id: &str, started_at: i64, worker_pid: Option<i64>) {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET started_at = ?2, worker_pid = ?3 WHERE id = ?1",
            params![id, started_at, worker_pid],
        )
        .unwrap();
        // Stale detection measures from the ACTIVE RUN's started_at
        // (hermes semantics), so backdate that too.
        conn.execute(
            "UPDATE task_runs SET started_at = ?2
              WHERE id = (SELECT current_run_id FROM tasks WHERE id = ?1)",
            params![id, started_at],
        )
        .unwrap();
    }

    #[test]
    fn detect_crashed_workers_reclaims_dead_pid() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "crashy");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        // A pid that cannot exist + started well past the grace window.
        backdate_task(&store, &task.id, KanbanStore::now() - 200, Some(2_147_483_647));

        let crashed = store.detect_crashed_workers().unwrap();
        assert_eq!(crashed, vec![task.id.clone()]);
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.status, "ready");
        assert!(t.worker_pid.is_none());
        assert_eq!(t.consecutive_failures, 1, "crash consumes the retry budget");
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert_eq!(run.outcome.as_deref(), Some("crashed"));
        let kinds: Vec<String> = store
            .events(&task.id)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&"crashed".to_string()));
    }

    #[test]
    fn detect_crashed_workers_respects_grace_and_live_pids() {
        let (_dir, store) = temp_store();
        // Freshly claimed (started now → inside grace): untouched.
        let fresh = make_task(&store, "fresh");
        store.ready_task(&fresh.id).unwrap();
        store.claim_task(&fresh.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        backdate_task(&store, &fresh.id, KanbanStore::now(), Some(2_147_483_647));
        // Live pid (our own process): untouched regardless of age.
        let live = make_task(&store, "live");
        store.ready_task(&live.id).unwrap();
        store.claim_task(&live.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        backdate_task(
            &store,
            &live.id,
            KanbanStore::now() - 200,
            Some(std::process::id() as i64),
        );

        assert!(store.detect_crashed_workers().unwrap().is_empty());
        assert_eq!(store.get_task(&fresh.id).unwrap().unwrap().status, "running");
        assert_eq!(store.get_task(&live.id).unwrap().unwrap().status, "running");
    }

    #[test]
    fn detect_stale_running_reclaims_heartbeatless_worker() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "silent worker");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        backdate_task(&store, &task.id, KanbanStore::now() - 500, None);

        // Disabled at 0; fresh thresholds skip it.
        assert!(store.detect_stale_running(0).unwrap().is_empty());
        assert!(store.detect_stale_running(10_000).unwrap().is_empty());

        let reclaimed = store.detect_stale_running(100).unwrap();
        assert_eq!(reclaimed, vec![task.id.clone()]);
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(t.status, "ready");
        assert_eq!(t.consecutive_failures, 0, "stale is NOT a worker failure");
        let run = store.latest_run(&task.id).unwrap().unwrap();
        assert_eq!(run.outcome.as_deref(), Some("stale"));
        let stale = store
            .events(&task.id)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "stale")
            .unwrap();
        assert_eq!(stale.payload["timeout_seconds"], 100);
    }

    #[test]
    fn detect_stale_running_skips_recent_heartbeat() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "chatty worker");
        store.ready_task(&task.id).unwrap();
        store.claim_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        store.heartbeat_task(&task.id, "host:1", DEFAULT_CLAIM_TTL_SECS).unwrap();
        backdate_task(&store, &task.id, KanbanStore::now() - 500, None);
        // Heartbeat is fresh → not stale even past the run timeout.
        assert!(store.detect_stale_running(100).unwrap().is_empty());
        assert_eq!(store.get_task(&task.id).unwrap().unwrap().status, "running");
    }

    #[test]
    fn repair_db_missing_ok_and_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        // Missing file.
        let report = repair_db(&dir.path().join("kanban.db"));
        assert_eq!(report.status, "missing");

        // Healthy store.
        let store = KanbanStore::open(dir.path().join("kanban.db")).unwrap();
        make_task(&store, "healthy");
        drop(store);
        let report = repair_db(&dir.path().join("kanban.db"));
        assert_eq!(report.status, "ok");
        assert!(report.backup_path.is_none());

        // Garbage bytes: sqlite refuses to open → corrupt + quarantined.
        let bad = dir.path().join("bad.db");
        std::fs::write(&bad, b"this is not a sqlite database at all....").unwrap();
        let report = repair_db(&bad);
        assert_eq!(report.status, "corrupt");
        assert!(report.backup_path.is_some());
        assert!(report.backup_path.unwrap().exists());
    }

    // ------------------------------------------------------------------
    // P139 — task workspaces (--workspace / --branch, resolve_workspace)
    // ------------------------------------------------------------------

    fn init_git_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        std::fs::write(dir.join("README.md"), "repo\n").unwrap();
        run(&["add", "README.md"]);
        run(&["commit", "-q", "-m", "init"]);
    }

    #[test]
    fn parse_workspace_flag_variants() {
        assert_eq!(parse_workspace_flag("").unwrap(), ("scratch".into(), None));
        assert_eq!(
            parse_workspace_flag("scratch").unwrap(),
            ("scratch".into(), None)
        );
        assert_eq!(
            parse_workspace_flag("worktree").unwrap(),
            ("worktree".into(), None)
        );
        assert_eq!(
            parse_workspace_flag("worktree:/repo/main").unwrap(),
            ("worktree".into(), Some("/repo/main".to_string()))
        );
        assert_eq!(
            parse_workspace_flag("dir:/tmp/ws").unwrap(),
            ("dir".into(), Some("/tmp/ws".to_string()))
        );
        assert!(parse_workspace_flag("dir:").is_err());
        assert!(parse_workspace_flag("worktree:   ").is_err());
        let err = parse_workspace_flag("nope").unwrap_err();
        assert!(err.contains("unknown --workspace value"), "{err}");
    }

    #[test]
    fn parse_branch_flag_validation() {
        assert_eq!(parse_branch_flag("feat/x").unwrap(), "feat/x");
        assert_eq!(parse_branch_flag("  padded  ").unwrap(), "padded");
        assert!(parse_branch_flag("   ").is_err());
        assert!(parse_branch_flag("-leading").is_err());
        assert!(parse_branch_flag("has space").is_err());
    }

    #[test]
    fn create_task_stores_workspace_fields() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "wt".into(),
                created_by: "tester".into(),
                workspace_kind: Some("worktree".into()),
                workspace_path: Some("/repo/main".into()),
                branch_name: Some("feat/x".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(task.workspace_kind, "worktree");
        assert_eq!(task.workspace_path.as_deref(), Some("/repo/main"));
        assert_eq!(task.branch_name.as_deref(), Some("feat/x"));
    }

    #[test]
    fn create_task_rejects_bad_workspace() {
        let (_dir, store) = temp_store();
        let err = store
            .create_task(&NewTask {
                title: "bad".into(),
                created_by: "tester".into(),
                workspace_kind: Some("dir".into()),
                workspace_path: Some("/tmp/x".into()),
                branch_name: Some("feat/x".into()),
                ..Default::default()
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("branch_name is only valid"),
            "{err}"
        );
        let err = store
            .create_task(&NewTask {
                title: "bad".into(),
                created_by: "tester".into(),
                workspace_kind: Some("cloud".into()),
                ..Default::default()
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("workspace_kind must be one of"),
            "{err}"
        );
    }

    #[test]
    fn resolve_workspace_scratch_and_dir() {
        let (dir, store) = temp_store();
        let scratch = make_task(&store, "scratchy");
        let (path, branch) = store.resolve_workspace(dir.path(), &scratch).unwrap();
        assert_eq!(path, workspaces_root(dir.path()).join(&scratch.id));
        assert!(path.is_dir());
        assert!(branch.is_none());

        let dir_task = store
            .create_task(&NewTask {
                title: "explicit dir".into(),
                created_by: "tester".into(),
                workspace_kind: Some("dir".into()),
                workspace_path: Some(dir.path().join("ws").to_string_lossy().to_string()),
                ..Default::default()
            })
            .unwrap();
        let (path, _) = store.resolve_workspace(dir.path(), &dir_task).unwrap();
        assert_eq!(path, dir.path().join("ws"));
        assert!(path.is_dir());

        // Relative dir paths are rejected (confused-deputy guard).
        let rel = store
            .create_task(&NewTask {
                title: "relative".into(),
                created_by: "tester".into(),
                workspace_kind: Some("dir".into()),
                workspace_path: Some("../escape".into()),
                ..Default::default()
            })
            .unwrap();
        let err = store.resolve_workspace(dir.path(), &rel).unwrap_err();
        assert!(err.contains("non-absolute"), "{err}");
    }

    #[test]
    fn resolve_workspace_worktree_via_board_workdir() {
        let (dir, store) = temp_store();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        store
            .set_board_workdir("default", Some(repo.to_str().unwrap()))
            .unwrap();

        let task = store
            .create_task(&NewTask {
                title: "wt via board".into(),
                created_by: "tester".into(),
                workspace_kind: Some("worktree".into()),
                ..Default::default()
            })
            .unwrap();
        // create_task fills workspace_path from the board default_workdir.
        assert_eq!(
            task.workspace_path.as_deref(),
            Some(repo.to_str().unwrap())
        );
        let (path, branch) = store.resolve_workspace(dir.path(), &task).unwrap();
        assert_eq!(path, repo.join(".worktrees").join(&task.id));
        assert!(path.is_dir());
        assert!(is_linked_worktree_checkout(&path));
        assert_eq!(branch.as_deref(), Some(format!("wt/{}", task.id).as_str()));

        // Re-resolving without a stored path anchors on the board again
        // and reuses the existing worktree.
        let bare = Task {
            workspace_path: None,
            ..task.clone()
        };
        let (again, _) = store.resolve_workspace(dir.path(), &bare).unwrap();
        assert_eq!(again, path);
    }

    #[test]
    fn resolve_workspace_worktree_sibling_gets_own_tree() {
        let (dir, store) = temp_store();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);

        let first = store
            .create_task(&NewTask {
                title: "first".into(),
                created_by: "tester".into(),
                workspace_kind: Some("worktree".into()),
                workspace_path: Some(repo.to_str().unwrap().to_string()),
                branch_name: Some("feat/shared".into()),
                ..Default::default()
            })
            .unwrap();
        let (first_path, first_branch) =
            store.resolve_workspace(dir.path(), &first).unwrap();
        assert_eq!(first_branch.as_deref(), Some("feat/shared"));
        assert_eq!(git_current_branch(&first_path).as_deref(), Some("feat/shared"));

        // A sibling inheriting the occupied checkout path must get its
        // own worktree instead of the other task's branch.
        let sibling = store
            .create_task(&NewTask {
                title: "sibling".into(),
                created_by: "tester".into(),
                workspace_kind: Some("worktree".into()),
                workspace_path: Some(first_path.to_str().unwrap().to_string()),
                ..Default::default()
            })
            .unwrap();
        let (sib_path, sib_branch) = store.resolve_workspace(dir.path(), &sibling).unwrap();
        assert_eq!(sib_path, repo.join(".worktrees").join(&sibling.id));
        assert_ne!(sib_path, first_path);
        assert!(sib_branch.unwrap().starts_with("wt/"));
    }

    #[test]
    fn dispatch_resolves_and_persists_workspace() {
        let (dir, store) = temp_store();
        let task = make_task(&store, "persist ws");
        store.ready_task(&task.id).unwrap();
        let mut seen: Option<PathBuf> = None;
        let result = store
            .dispatch_once(
                dir.path(),
                false,
                |_task, ws| {
                    seen = ws.map(|p| p.to_path_buf());
                    Ok(Some(42))
                },
                Some(2),
                false,
                2,
                0,
            )
            .unwrap();
        assert_eq!(result.spawned.len(), 1);
        let expected = workspaces_root(dir.path()).join(&task.id);
        assert_eq!(seen.as_ref(), Some(&expected));
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(
            t.workspace_path.as_deref(),
            Some(expected.to_str().unwrap())
        );
    }

    #[test]
    fn dispatch_workspace_error_counts_as_spawn_failure() {
        let (dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "bad dir".into(),
                created_by: "tester".into(),
                workspace_kind: Some("dir".into()),
                workspace_path: Some("relative/path".into()),
                ..Default::default()
            })
            .unwrap();
        store.ready_task(&task.id).unwrap();
        let result = store
            .dispatch_once(
                dir.path(),
                false,
                |_, _| Ok(Some(1)),
                Some(2),
                false,
                2,
                0,
            )
            .unwrap();
        assert_eq!(result.spawn_failed, vec![task.id.clone()]);
        assert!(result.spawned.is_empty());
        let t = store.get_task(&task.id).unwrap().unwrap();
        assert!(t.last_failure_error.unwrap().starts_with("workspace:"));
    }

    #[test]
    fn dispatch_worktrees_flag_upgrades_scratch_tasks() {
        let (dir, store) = temp_store();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        store
            .set_board_workdir("default", Some(repo.to_str().unwrap()))
            .unwrap();

        let task = make_task(&store, "legacy worktrees");
        store.ready_task(&task.id).unwrap();
        let result = store
            .dispatch_once(dir.path(), true, |_, _| Ok(Some(7)), Some(2), false, 2, 0)
            .unwrap();
        assert_eq!(result.spawned.len(), 1);
        let t = store.get_task(&task.id).unwrap().unwrap();
        let expected = repo.join(".worktrees").join(&task.id);
        assert_eq!(
            t.workspace_path.as_deref(),
            Some(expected.to_str().unwrap())
        );
        assert!(expected.is_dir());
        assert_eq!(
            t.branch_name.as_deref(),
            Some(format!("wt/{}", task.id).as_str())
        );
    }
}
