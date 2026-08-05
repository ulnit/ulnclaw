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
        })
    }

    const TASK_COLUMNS: &'static str = "id, board, title, body, assignee, status, priority, \
        created_by, created_at, started_at, completed_at, tenant, model, result, \
        claim_lock, claim_expires, last_heartbeat_at";

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
}
