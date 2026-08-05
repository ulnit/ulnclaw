//! Kanban task engine — port of the hermes `hermes_cli/kanban_db.py`
//! core (boards, tasks, claim locks, comments, events) backing
//! `ulnclaw kanban`.
//!
//! The store lives in `<home>/kanban.db` (hermes keeps a per-profile
//! board). Lifecycle transitions append `task_events` rows and the CLI
//! fires the `kanban_task_claimed` / `kanban_task_completed` /
//! `kanban_task_blocked` hooks that the plugin runtime validates
//! (P95 wired the events; this engine emits them).

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{AgentError, Result};

/// Hermes task statuses (`_STATUS_ICONS` keys).
pub const STATUSES: &[&str] = &[
    "todo", "ready", "running", "scheduled", "blocked", "done", "archived",
];

/// Status glyphs (hermes `_STATUS_ICONS`).
pub fn status_icon(status: &str) -> &'static str {
    match status {
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
}

/// Worker brief for a dispatcher-spawned task (hermes spawns
/// `hermes chat -q "work kanban task <id>"`; the ulnclaw one-shot is
/// `ulnclaw run`).
pub fn worker_prompt(task: &Task) -> String {
    format!(
        "You are a kanban worker for task {} ({}). Start by calling kanban_show \
         (task_id defaults to your own task) to read the full brief, then do the \
         work. When finished you MUST call kanban_complete with a result summary; \
         if you cannot proceed, call kanban_block with the reason. Use \
         kanban_heartbeat to report progress on long steps.",
        task.id, task.title
    )
}

/// Dispatch-time spawn: prepare an isolated worktree (when enabled and
/// the cwd is a git repo), then spawn the detached worker in it.
pub fn dispatch_spawn(
    home: &Path,
    use_worktrees: bool,
    task: &Task,
) -> std::result::Result<Option<i64>, String> {
    let workdir = if use_worktrees {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        prepare_worktree(&cwd, task)?
    } else {
        None
    };
    spawn_worker(home, task, workdir.as_deref())
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
    cmd.arg("run").arg(worker_prompt(task));
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

/// One parallel worker card in a swarm (hermes `SwarmWorkerSpec`).
#[derive(Debug, Clone)]
pub struct SwarmWorkerSpec {
    pub assignee: String,
    pub title: String,
    pub body: String,
    pub priority: i64,
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
        })?;
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
}

pub struct KanbanStore {
    conn: Mutex<Connection>,
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
                last_heartbeat_at INTEGER
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
            );",
        )
        .map_err(db_error("schema"))?;
        // Additive migration: pre-P122 stores lack the worker_pid column.
        let has_worker_pid: bool = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(tasks)")
                .map_err(db_error("migrate"))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(db_error("migrate"))?;
            let found = rows.flatten().any(|name| name == "worker_pid");
            found
        };
        if !has_worker_pid {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN worker_pid INTEGER;")
                .map_err(db_error("migrate worker_pid"))?;
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
        let id = Self::new_task_id();
        let board = self.current_board()?;
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (id, board, title, body, assignee, status, priority, \
             created_by, created_at, tenant, model) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'todo', ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                board,
                task.title.trim(),
                task.body,
                task.assignee,
                task.priority,
                task.created_by,
                now,
                task.tenant,
                task.model,
            ],
        )
        .map_err(db_error("create task"))?;
        drop(conn);
        self.append_event(&id, "created", serde_json::json!({ "board": board }))?;
        self.get_task(&id)?
            .ok_or_else(|| AgentError::session("kanban: task vanished after create"))
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
        })
    }

    const TASK_COLUMNS: &'static str = "id, board, title, body, assignee, status, priority, \
        created_by, created_at, started_at, completed_at, tenant, model, result, \
        claim_lock, claim_expires, last_heartbeat_at, worker_pid";

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

    fn append_event(&self, task_id: &str, kind: &str, payload: Value) -> Result<()> {
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
        drop(conn);
        if updated == 0 {
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
        self.append_event(
            id,
            "claimed",
            serde_json::json!({ "lock": lock, "expires": expires, "claimer": claimer }),
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
            ", completed_at = ?3, result = ?4, claim_lock = NULL, claim_expires = NULL",
            vec![Box::new(now), Box::new(result.map(|r| r.to_string()))],
        )?;
        Ok(task)
    }

    /// Block a task with a reason (hermes `block_task`).
    pub fn block_task(&self, id: &str, reason: &str) -> Result<Task> {
        self.transition(
            id,
            &["todo", "ready", "running", "scheduled"],
            "blocked",
            "blocked",
            serde_json::json!({ "reason": reason }),
            ", claim_lock = NULL, claim_expires = NULL",
            vec![],
        )
    }

    /// blocked → ready (hermes `unblock_task`).
    pub fn unblock_task(&self, id: &str) -> Result<Task> {
        self.transition(
            id,
            &["blocked"],
            "ready",
            "unblocked",
            serde_json::json!({}),
            "",
            vec![],
        )
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

    /// Consecutive spawn failures recorded for `task_id` (hermes counts
    /// them to auto-block unfixable tasks).
    fn spawn_failure_streak(&self, task_id: &str) -> Result<usize> {
        let events = self.events(task_id)?;
        let mut streak = 0usize;
        for event in events.iter().rev() {
            match event.kind.as_str() {
                "spawn_failed" => streak += 1,
                "spawned" | "released" | "ready" | "created" => break,
                _ => {}
            }
        }
        Ok(streak)
    }

    /// Record the pid of a dispatcher-spawned worker.
    pub fn set_worker_pid(&self, id: &str, pid: Option<i64>) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE tasks SET worker_pid = ?2 WHERE id = ?1",
                params![id, pid],
            )
            .map_err(db_error("worker_pid"))?;
        Ok(())
    }

    /// Run one dispatcher tick (hermes `dispatch_once`, scoped port):
    /// 1. reclaim stale claims, 2. promote parent-done todos, 3. spawn
    /// ready tasks (priority desc, oldest first) up to the live
    /// concurrency cap `max_spawn` (counting already-running tasks).
    /// `spawn` returns the worker pid; failures are counted per task and
    /// after `failure_limit` consecutive failures the task is auto-blocked
    /// with the last error (hermes DEFAULT_FAILURE_LIMIT = 2).
    pub fn dispatch_once<F>(
        &self,
        mut spawn: F,
        max_spawn: Option<usize>,
        dry_run: bool,
        failure_limit: usize,
    ) -> Result<DispatchResult>
    where
        F: FnMut(&Task) -> std::result::Result<Option<i64>, String>,
    {
        let mut result = DispatchResult::default();
        result.reclaimed = self.release_stale_claims()?;
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
            match spawn(&task) {
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
                    let streak = self.spawn_failure_streak(&id)?;
                    if streak >= failure_limit {
                        self.block_task(
                            &id,
                            &format!("dispatcher: spawn failed {streak}x — {err}"),
                        )?;
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
        let (_dir, store) = temp_store();
        let first = make_task(&store, "first");
        let second = make_task(&store, "second");
        store.ready_task(&first.id).unwrap();
        store.ready_task(&second.id).unwrap();

        let result = store
            .dispatch_once(|_| Ok(Some(1234)), Some(1), false, 2)
            .unwrap();
        assert_eq!(result.spawned.len(), 1);
        assert_eq!(result.spawned[0].0, first.id);
        assert_eq!(result.skipped_capped, vec![second.id.clone()]);
        let spawned_task = store.get_task(&first.id).unwrap().unwrap();
        assert_eq!(spawned_task.status, "running");
        assert_eq!(spawned_task.worker_pid, Some(1234));

        // Second tick with a higher cap picks up the remaining task.
        let result = store
            .dispatch_once(|_| Ok(Some(5678)), Some(2), false, 2)
            .unwrap();
        assert_eq!(result.spawned.len(), 1);
        assert_eq!(result.spawned[0].0, second.id);
    }

    #[test]
    fn dispatch_dry_run_spawns_nothing() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "probe");
        store.ready_task(&task.id).unwrap();
        let result = store
            .dispatch_once(|_| panic!("dry run must not spawn"), None, true, 2)
            .unwrap();
        assert_eq!(result.would_spawn, vec![task.id.clone()]);
        assert!(result.spawned.is_empty());
        assert_eq!(store.get_task(&task.id).unwrap().unwrap().status, "ready");
    }

    #[test]
    fn dispatch_auto_blocks_after_repeated_spawn_failures() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "doomed");
        store.ready_task(&task.id).unwrap();

        // First failure: recorded, still ready-ish for retry.
        let result = store
            .dispatch_once(|_| Err("boom".into()), None, false, 2)
            .unwrap();
        assert_eq!(result.spawn_failed, vec![task.id.clone()]);
        assert!(result.auto_blocked.is_empty());

        // Second consecutive failure trips the limit → blocked.
        let result = store
            .dispatch_once(|_| Err("boom again".into()), None, false, 2)
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
            },
            SwarmWorkerSpec {
                assignee: "bob".into(),
                title: "Draft".into(),
                body: "write draft".into(),
                priority: 0,
            },
        ];
        let created = store
            .create_swarm("Ship a report", &workers, "carol", "dave", "")
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
}
