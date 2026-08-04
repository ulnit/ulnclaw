//! SQLite session storage — port of hermes_state.py / hermes_state_common.py
//!
//! Schema follows the hermes SCHEMA_SQL (sessions + messages + system_prompts
//! + state_meta + async_delegations) with FTS5 full-text search over message
//! content and a LIKE fallback when FTS5 is unavailable.

use crate::error::{AgentError, Result};
use crate::provider::{Message, Role, ToolCall};
use crate::session::{Session, SessionMetadata, SessionStore};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Core schema — adapted from hermes_state_common.SCHEMA_SQL.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS system_prompts (
    hash TEXT PRIMARY KEY,
    prompt TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL DEFAULT 'cli',
    user_id TEXT,
    session_key TEXT,
    model TEXT,
    system_prompt TEXT,
    system_prompt_hash TEXT,
    parent_session_id TEXT,
    started_at REAL NOT NULL,
    ended_at REAL,
    end_reason TEXT,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cwd TEXT,
    title TEXT,
    last_activity_at REAL,
    archived INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    role TEXT NOT NULL,
    content TEXT,
    tool_call_id TEXT,
    tool_calls TEXT,
    tool_name TEXT,
    timestamp REAL NOT NULL,
    token_count INTEGER,
    finish_reason TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS state_meta (
    key TEXT PRIMARY KEY,
    value TEXT
);

CREATE TABLE IF NOT EXISTS async_delegations (
    delegation_id TEXT PRIMARY KEY,
    origin_session TEXT NOT NULL,
    parent_session_id TEXT,
    state TEXT NOT NULL,
    dispatched_at REAL NOT NULL,
    completed_at REAL,
    updated_at REAL NOT NULL,
    result_json TEXT,
    task_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source);
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id, id);
"#;

/// FTS5 virtual table (created when supported by the SQLite build).
const FTS_SQL: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content,
    content='messages',
    content_rowid='id',
    tokenize='unicode61'
);
"#;

/// One session row exposed by the gateway API.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionRow {
    pub id: String,
    pub source: String,
    pub model: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub parent_session_id: Option<String>,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub end_reason: Option<String>,
    pub message_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// A SQLite-backed session store.
pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
    path: PathBuf,
    has_fts: bool,
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl SqliteSessionStore {
    /// Open (or create) the state database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(&path)
            .map_err(|e| AgentError::session(format!("open {}: {}", path.display(), e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| AgentError::session(format!("pragmas: {}", e)))?;
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| AgentError::session(format!("schema: {}", e)))?;

        // FTS5 probe — same pattern as hermes_state_schema's _fts5_available.
        let has_fts = conn
            .execute_batch(FTS_SQL)
            .map(|_| true)
            .unwrap_or(false);

        let store = Self {
            conn: Mutex::new(conn),
            path,
            has_fts,
        };
        store.init_meta()?;
        Ok(store)
    }

    /// Open the default state DB at `<home>/state.db`.
    pub fn open_default() -> Result<Self> {
        let path = crate::config::ulnclaw_home().join("state.db");
        Self::open(path)
    }

    fn init_meta(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let version: Option<i64> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
            .optional()
            .map_err(|e| AgentError::session(e.to_string()))?;
        if version.is_none() {
            conn.execute("INSERT INTO schema_version (version) VALUES (?1)", params![1])
                .map_err(|e| AgentError::session(e.to_string()))?;
        }
        Ok(())
    }

    /// Database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether FTS5 full-text search is available.
    pub fn has_fts(&self) -> bool {
        self.has_fts
    }

    /// Create a new session row, returning its id.
    pub fn create_session(&self, source: &str, model: Option<&str>, cwd: Option<&str>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO sessions (id, source, model, cwd, started_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, source, model, cwd, now_secs()],
        )
        .map_err(|e| AgentError::session(format!("create session: {}", e)))?;
        Ok(id)
    }

    /// Create a child session (delegation lineage).
    pub fn create_child_session(
        &self,
        parent_id: &str,
        source: &str,
        model: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO sessions (id, source, model, parent_session_id, started_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, source, model, parent_id, now_secs()],
        )
        .map_err(|e| AgentError::session(format!("create child session: {}", e)))?;
        Ok(id)
    }

    /// Create a session with a caller-chosen id (fork API). `parent` links
    /// the new session into an existing lineage.
    pub fn create_named_session(
        &self,
        id: &str,
        source: &str,
        model: Option<&str>,
        parent: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO sessions (id, source, model, parent_session_id, started_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, source, model, parent, now_secs()],
        )
        .map_err(|e| AgentError::session(format!("create named session: {}", e)))?;
        Ok(())
    }

    /// Append one message to a session.
    pub fn append_message(&self, session_id: &str, message: &Message) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let tool_calls = message
            .tool_calls
            .as_ref()
            .map(|calls| serde_json::to_string(calls))
            .transpose()
            .map_err(|e| AgentError::session(e.to_string()))?;
        let role = message.role.to_string();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_call_id, tool_calls, tool_name, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                role,
                message.content,
                message.tool_call_id,
                tool_calls,
                message.name,
                now_secs(),
            ],
        )
        .map_err(|e| AgentError::session(format!("append message: {}", e)))?;
        let msg_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "UPDATE sessions SET message_count = message_count + 1, last_activity_at = ?2
             WHERE id = ?1",
            params![session_id, now_secs()],
        )
        .ok();
        if self.has_fts {
            // External-content FTS: index the content manually.
            conn.execute(
                "INSERT INTO messages_fts (rowid, content) VALUES (?1, ?2)",
                params![msg_id, message.content],
            )
            .ok();
        }
        Ok(())
    }

    /// Load all active messages of a session as `Message` values.
    pub fn load_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT role, content, tool_call_id, tool_calls, tool_name
                 FROM messages WHERE session_id = ?1 AND active = 1 ORDER BY id",
            )
            .map_err(|e| AgentError::session(e.to_string()))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| AgentError::session(e.to_string()))?;
        let mut messages = Vec::new();
        for row in rows {
            let (role, content, tool_call_id, tool_calls, name) =
                row.map_err(|e| AgentError::session(e.to_string()))?;
            let role = match role.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => Role::Tool,
            };
            let tool_calls: Option<Vec<ToolCall>> = tool_calls
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| AgentError::session(e.to_string()))?;
            messages.push(Message {
                role,
                content,
                tool_calls,
                tool_call_id,
                name,
            });
        }
        Ok(messages)
    }

    /// Full-text search over all messages. Returns (session_id, snippet, rank).
    pub fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let mut results = Vec::new();
        if self.has_fts {
            let fts_query = query
                .split_whitespace()
                .map(|token| format!("\"{}\"", token.replace('"', "")))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT m.session_id, snippet(messages_fts, 0, '[', ']', '…', 12)
                 FROM messages_fts
                 JOIN messages m ON m.id = messages_fts.rowid
                 WHERE messages_fts MATCH ?1
                 ORDER BY rank LIMIT ?2"
            );
            match conn.prepare(&sql) {
                Ok(mut stmt) => {
                    let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    });
                    if let Ok(rows) = rows {
                        for row in rows.flatten() {
                            results.push(row);
                        }
                        return Ok(results);
                    }
                }
                Err(_) => {}
            }
        }
        // LIKE fallback
        let pattern = format!("%{}%", query);
        let mut stmt = conn
            .prepare(
                "SELECT session_id, substr(content, 1, 200)
                 FROM messages WHERE content LIKE ?1 AND active = 1
                 ORDER BY timestamp DESC LIMIT ?2",
            )
            .map_err(|e| AgentError::session(e.to_string()))?;
        for row in stmt
            .query_map(params![pattern, limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| AgentError::session(e.to_string()))?
        {
            if let Ok(pair) = row {
                results.push(pair);
            }
        }
        Ok(results)
    }

    /// Update session token/usage counters.
    pub fn update_usage(
        &self,
        session_id: &str,
        input_tokens: u32,
        output_tokens: u32,
        tool_calls: u32,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET input_tokens = input_tokens + ?2,
                output_tokens = output_tokens + ?3,
                tool_call_count = tool_call_count + ?4
             WHERE id = ?1",
            params![session_id, input_tokens, output_tokens, tool_calls],
        )
        .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(())
    }

    /// Ensure a session row exists (create it if missing). Used by the
    /// gateway to resume a caller-supplied session id.
    pub fn ensure_session(
        &self,
        id: &str,
        source: &str,
        model: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO sessions (id, source, model, cwd, started_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET last_activity_at = excluded.last_activity_at",
            params![id, source, model, cwd, now_secs()],
        )
        .map_err(|e| AgentError::session(format!("ensure session: {}", e)))?;
        Ok(())
    }

    /// One session row, if present.
    pub fn get_session_row(&self, session_id: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.query_row(
            "SELECT id, source, model, title, cwd, parent_session_id, started_at,
                    ended_at, end_reason, message_count, input_tokens, output_tokens
             FROM sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    model: row.get(2)?,
                    title: row.get(3)?,
                    cwd: row.get(4)?,
                    parent_session_id: row.get(5)?,
                    started_at: row.get(6)?,
                    ended_at: row.get(7)?,
                    end_reason: row.get(8)?,
                    message_count: row.get(9)?,
                    input_tokens: row.get(10)?,
                    output_tokens: row.get(11)?,
                })
            },
        )
        .optional()
        .map_err(|e| AgentError::session(e.to_string()))
    }

    /// Recent sessions, newest first.
    pub fn list_session_rows(&self, limit: usize) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source, model, title, cwd, parent_session_id, started_at,
                        ended_at, end_reason, message_count, input_tokens, output_tokens
                 FROM sessions ORDER BY started_at DESC LIMIT ?1",
            )
            .map_err(|e| AgentError::session(e.to_string()))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    model: row.get(2)?,
                    title: row.get(3)?,
                    cwd: row.get(4)?,
                    parent_session_id: row.get(5)?,
                    started_at: row.get(6)?,
                    ended_at: row.get(7)?,
                    end_reason: row.get(8)?,
                    message_count: row.get(9)?,
                    input_tokens: row.get(10)?,
                    output_tokens: row.get(11)?,
                })
            })
            .map_err(|e| AgentError::session(e.to_string()))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| AgentError::session(e.to_string()))?);
        }
        Ok(sessions)
    }

    /// Mark a session ended.
    pub fn end_session(&self, session_id: &str, reason: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET ended_at = ?2, end_reason = ?3 WHERE id = ?1",
            params![session_id, now_secs(), reason],
        )
        .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(())
    }

    /// Set (or clear, with an empty string) a session title. Titles must not
    /// contain newlines or NUL bytes.
    pub fn set_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        if title.contains(['\r', '\n', '\0']) {
            return Err(AgentError::session("invalid session title"));
        }
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1",
            params![session_id, title],
        )
        .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(())
    }

    /// Lock a session to a specific model (gateway model-lock API). The
    /// locked model is inherited by forks (via the `model` column) and
    /// survives `ensure_session` resumes.
    pub fn set_session_model(&self, session_id: &str, model: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET model = ?2 WHERE id = ?1",
            params![session_id, model],
        )
        .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(())
    }

    /// Set/get state metadata.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO state_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.query_row(
            "SELECT value FROM state_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| AgentError::session(e.to_string()))
    }
}

impl SessionStore for SqliteSessionStore {
    fn save_session(&self, session: &Session) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        conn.execute(
            "INSERT INTO sessions (id, source, model, parent_session_id, started_at, last_activity_at, title)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                last_activity_at = excluded.last_activity_at,
                title = COALESCE(excluded.title, sessions.title)",
            params![
                session.id,
                session.metadata.platform.as_deref().unwrap_or("cli"),
                session.metadata.model,
                session.parent_id,
                session.created_at_ms as f64 / 1000.0,
                session.updated_at_ms as f64 / 1000.0,
                serde_json::to_value(&session.metadata)
                    .ok()
                    .and_then(|v| v.get("title").and_then(|t| t.as_str().map(String::from))),
            ],
        )
        .map_err(|e| AgentError::session(format!("save session: {}", e)))?;
        drop(conn);
        // Persist messages that are not stored yet (naive full rewrite for the
        // generic SessionStore API; append_message is the fast path).
        let existing = self.load_messages(&session.id)?.len();
        for message in session.messages.iter().skip(existing) {
            self.append_message(&session.id, message)?;
        }
        Ok(())
    }

    fn load_session(&self, session_id: &str) -> Result<Option<Session>> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        let row = conn
            .query_row(
                "SELECT id, source, model, parent_session_id, started_at, last_activity_at
                 FROM sessions WHERE id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, f64>(4)?,
                        row.get::<_, Option<f64>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AgentError::session(e.to_string()))?;
        drop(conn);
        let Some((id, source, model, parent, started, activity)) = row else {
            return Ok(None);
        };
        let messages = self.load_messages(session_id)?;
        Ok(Some(Session {
            id,
            conversation_id: session_id.to_string(),
            messages,
            created_at_ms: (started * 1000.0) as i64,
            updated_at_ms: (activity.unwrap_or(started) * 1000.0) as i64,
            parent_id: parent,
            metadata: SessionMetadata {
                platform: Some(source),
                model,
                ..Default::default()
            },
        }))
    }

    fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        let mut ids: Vec<String> = Vec::new();
        {
            let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM sessions ORDER BY last_activity_at DESC LIMIT ?1",
                )
                .map_err(|e| AgentError::session(e.to_string()))?;
            let rows = stmt
                .query_map(params![limit as i64], |row| row.get::<_, String>(0))
                .map_err(|e| AgentError::session(e.to_string()))?;
            for row in rows {
                if let Ok(id) = row {
                    ids.push(id);
                }
            }
        }
        let mut sessions = Vec::new();
        for id in ids {
            if let Some(session) = self.load_session(&id)? {
                sessions.push(session);
            }
        }
        Ok(sessions)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| AgentError::session(e.to_string()))?;
        if self.has_fts {
            conn.execute(
                "DELETE FROM messages_fts WHERE rowid IN
                 (SELECT id FROM messages WHERE session_id = ?1)",
                params![session_id],
            )
            .ok();
        }
        conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])
            .ok();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
            .map_err(|e| AgentError::session(e.to_string()))?;
        Ok(())
    }

    fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<Session>> {
        let hits = self.search_messages(query, limit * 3)?;
        let mut seen = std::collections::HashSet::new();
        let mut sessions = Vec::new();
        for (session_id, _) in hits {
            if seen.insert(session_id.clone()) {
                if let Some(session) = self.load_session(&session_id)? {
                    sessions.push(session);
                    if sessions.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteSessionStore::open(dir.path().join("state.db")).unwrap();
        let sid = store.create_session("cli", Some("test-model"), Some("/tmp")).unwrap();
        store
            .append_message(
                &sid,
                &Message {
                    role: Role::User,
                    content: Some("hello world from ulnclaw".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            )
            .unwrap();
        let messages = store.load_messages(&sid).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("hello world from ulnclaw"));

        let results = store.search_messages("ulnclaw", 5).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, sid);

        let session = store.load_session(&sid).unwrap().unwrap();
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.metadata.model.as_deref(), Some("test-model"));
    }

    #[test]
    fn test_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteSessionStore::open(dir.path().join("state.db")).unwrap();
        let parent = store.create_session("cli", None, None).unwrap();
        let child = store.create_child_session(&parent, "delegate", None).unwrap();
        let child_session = store.load_session(&child).unwrap().unwrap();
        assert_eq!(child_session.parent_id.as_deref(), Some(parent.as_str()));
    }
}
