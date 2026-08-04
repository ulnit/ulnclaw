//! Background (fire-and-forget) delegation — port of hermes
//! `tools/async_delegation.py` core semantics.
//!
//! `delegate_task` with `background: true` dispatches the fan-out as a
//! detached task and returns immediately with a `delegation_id`. Per-child
//! results stream into live log files (`<home>/cache/delegation/live/<id>/
//! task-<n>.log`); when ALL children finish a consolidated report is written
//! (`result.json`) and delivered back to the parent conversation:
//!
//! - REPL turns drain the process-local completion queue between prompts
//!   (hermes CLI drain with positive-ownership filtering by session key).
//! - Gateway session chats drain the same queue before each turn.
//!
//! Hermes' durable sqlite registry (crash recovery, delivery claims,
//! multiplex-profile routing) is not ported: the registry lives for the
//! process lifetime and the files under `cache/delegation/live/` are the
//! durable artifact.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::tools::context::SubAgentRunner;

/// Status of a background delegation.
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";

#[derive(Debug, Clone)]
pub struct DelegationRecord {
    pub id: String,
    pub parent_session_key: String,
    pub tasks: usize,
    pub status: String,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
    pub log_dir: PathBuf,
}

/// A finished delegation waiting to be injected into the parent session.
#[derive(Debug, Clone)]
pub struct Completion {
    pub delegation_id: String,
    pub session_key: String,
    /// Consolidated, model-ready report text.
    pub message: String,
    pub result: serde_json::Value,
    pub finished_ms: i64,
}

#[derive(Default)]
struct DelegationState {
    records: HashMap<String, DelegationRecord>,
    completions: VecDeque<Completion>,
}

fn state() -> &'static Mutex<DelegationState> {
    static STATE: OnceLock<Mutex<DelegationState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(DelegationState::default()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Live directory root: `<home>/cache/delegation/live` (hermes layout).
pub fn live_root(home: &Path) -> PathBuf {
    home.join("cache").join("delegation").join("live")
}

/// Dispatch a background fan-out. Returns the record immediately; the
/// children run on a detached tokio task bounded by `max_concurrent`.
pub fn dispatch_background_delegation(
    runner: Arc<dyn SubAgentRunner>,
    tasks: Vec<(String, String)>,
    parent_session_key: String,
    home: PathBuf,
    max_concurrent: usize,
) -> Result<DelegationRecord, String> {
    if tasks.is_empty() {
        return Err("no tasks to delegate".to_string());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let log_dir = live_root(&home).join(&id);
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| format!("cannot create delegation log dir: {e}"))?;

    let record = DelegationRecord {
        id: id.clone(),
        parent_session_key: parent_session_key.clone(),
        tasks: tasks.len(),
        status: STATUS_RUNNING.to_string(),
        created_ms: now_ms(),
        finished_ms: None,
        log_dir: log_dir.clone(),
    };
    state().lock().unwrap().records.insert(id.clone(), record.clone());

    tokio::spawn(async move {
        let results = run_batch(runner, tasks, &log_dir, max_concurrent.max(1)).await;
        finalize_delegation(&id, &parent_session_key, &log_dir, results);
    });

    Ok(record)
}

/// Run every child (bounded concurrency), writing per-task live logs.
async fn run_batch(
    runner: Arc<dyn SubAgentRunner>,
    tasks: Vec<(String, String)>,
    log_dir: &Path,
    max_concurrent: usize,
) -> Vec<serde_json::Value> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    let mut handles = Vec::new();
    for (idx, (goal, context)) in tasks.into_iter().enumerate() {
        let runner = runner.clone();
        let permit = Arc::clone(&semaphore);
        let log_path = log_dir.join(format!("task-{}.log", idx + 1));
        handles.push(tokio::spawn(async move {
            let _permit = permit.acquire().await.expect("semaphore open");
            let started = now_ms();
            let outcome = runner.run_subagent(&goal, &context).await;
            let elapsed_ms = now_ms() - started;
            let entry = match &outcome {
                Ok(answer) => json!({
                    "task": goal,
                    "status": "completed",
                    "elapsed_ms": elapsed_ms,
                    "result": answer,
                }),
                Err(e) => json!({
                    "task": goal,
                    "status": "error",
                    "elapsed_ms": elapsed_ms,
                    "error": e.to_string(),
                }),
            };
            let mut log = String::new();
            log.push_str(&format!("task: {}\n", goal));
            if !context.is_empty() {
                log.push_str(&format!("context: {}\n", context));
            }
            log.push_str(&format!("status: {}\n", entry["status"]));
            log.push_str(&serde_json::to_string_pretty(&entry).unwrap_or_default());
            log.push('\n');
            let _ = std::fs::write(&log_path, log);
            entry
        }));
    }

    let mut results = Vec::new();
    for (idx, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(entry) => results.push(entry),
            Err(e) => results.push(json!({
                "task": format!("task-{}", idx + 1),
                "status": "error",
                "error": format!("join: {e}"),
            })),
        }
    }
    results
}

/// Write the consolidated result, update the registry and enqueue the
/// completion for delivery to the parent session.
fn finalize_delegation(
    id: &str,
    parent_session_key: &str,
    log_dir: &Path,
    results: Vec<serde_json::Value>,
) {
    let failed = results
        .iter()
        .filter(|r| r["status"] != "completed")
        .count();
    let status = if failed == results.len() {
        STATUS_FAILED
    } else {
        STATUS_COMPLETED
    };
    let finished = now_ms();

    let consolidated = json!({
        "delegation_id": id,
        "status": status,
        "subagents": results.len(),
        "failed": failed,
        "results": results,
    });
    let _ = std::fs::write(
        log_dir.join("result.json"),
        serde_json::to_string_pretty(&consolidated).unwrap_or_default(),
    );
    let _ = std::fs::write(log_dir.join("DONE"), format!("{}\n", status));

    let message = format_consolidated_report(id, &consolidated);

    let mut guard = state().lock().unwrap();
    if let Some(record) = guard.records.get_mut(id) {
        record.status = status.to_string();
        record.finished_ms = Some(finished);
    }
    guard.completions.push_back(Completion {
        delegation_id: id.to_string(),
        session_key: parent_session_key.to_string(),
        message,
        result: consolidated,
        finished_ms: finished,
    });
}

/// Model-facing text for a finished delegation (hermes consolidated block).
pub fn format_consolidated_report(delegation_id: &str, consolidated: &serde_json::Value) -> String {
    let subagents = consolidated["subagents"].as_u64().unwrap_or(0);
    let failed = consolidated["failed"].as_u64().unwrap_or(0);
    let mut out = format!(
        "Background delegation {} finished: {} subagent(s), {} failed.\n\n",
        delegation_id, subagents, failed
    );
    if let Some(results) = consolidated["results"].as_array() {
        for (i, entry) in results.iter().enumerate() {
            let task = entry["task"].as_str().unwrap_or("");
            let status = entry["status"].as_str().unwrap_or("error");
            out.push_str(&format!("{}. [{}] {}\n", i + 1, status, task));
            if status == "completed" {
                let result = entry["result"].as_str().unwrap_or("");
                out.push_str(result.trim_end());
                out.push('\n');
            } else {
                out.push_str(&format!(
                    "error: {}\n",
                    entry["error"].as_str().unwrap_or("unknown")
                ));
            }
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// Claim all queued completions owned by `session_key` (hermes
/// positive-ownership drain). Completions for other sessions stay queued.
pub fn drain_completions(session_key: &str) -> Vec<Completion> {
    if session_key.is_empty() {
        return Vec::new();
    }
    let mut guard = state().lock().unwrap();
    let mut claimed = Vec::new();
    let mut rest = VecDeque::new();
    while let Some(completion) = guard.completions.pop_front() {
        if completion.session_key == session_key {
            claimed.push(completion);
        } else {
            rest.push_back(completion);
        }
    }
    guard.completions = rest;
    claimed
}

/// Snapshot of all live/finished delegations in this process.
pub fn list_delegations() -> Vec<DelegationRecord> {
    let guard = state().lock().unwrap();
    let mut records: Vec<DelegationRecord> = guard.records.values().cloned().collect();
    records.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
    records
}

/// Fetch a single delegation record by id.
pub fn get_delegation(id: &str) -> Option<DelegationRecord> {
    state().lock().unwrap().records.get(id).cloned()
}

/// Read the consolidated result for a finished delegation, if present.
pub fn read_result(home: &Path, id: &str) -> Option<serde_json::Value> {
    let path = live_root(home).join(id).join("result.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AgentError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeRunner {
        delay_ms: u64,
        fail: bool,
        runs: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl SubAgentRunner for FakeRunner {
        async fn run_subagent(&self, goal: &str, _context: &str) -> crate::error::Result<String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            if self.fail {
                Err(AgentError::tool("boom"))
            } else {
                Ok(format!("answer to: {goal}"))
            }
        }
    }

    fn unique_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ulnclaw-asyncdel-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn wait_for_completion(key: &str, timeout_ms: u64) -> Vec<Completion> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let drained = drain_completions(key);
            if !drained.is_empty() {
                return drained;
            }
            if std::time::Instant::now() > deadline {
                return Vec::new();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn dispatch_runs_children_and_delivers_completion() {
        let home = unique_home("basic");
        let runner: Arc<dyn SubAgentRunner> = Arc::new(FakeRunner {
            delay_ms: 5,
            fail: false,
            runs: AtomicUsize::new(0),
        });
        let record = dispatch_background_delegation(
            runner,
            vec![
                ("goal A".to_string(), "ctx A".to_string()),
                ("goal B".to_string(), String::new()),
            ],
            "session-owner".to_string(),
            home.clone(),
            2,
        )
        .unwrap();

        let completions = wait_for_completion("session-owner", 5000).await;
        assert_eq!(completions.len(), 1);
        let completion = &completions[0];
        assert_eq!(completion.delegation_id, record.id);
        assert!(completion.message.contains("goal A"));
        assert!(completion.message.contains("answer to: goal A"));
        assert!(completion.message.contains("answer to: goal B"));
        assert_eq!(completion.result["status"], "completed");
        assert_eq!(completion.result["failed"], 0);

        // Files on disk: per-task logs + result.json + DONE marker.
        let dir = live_root(&home).join(&record.id);
        assert!(dir.join("task-1.log").exists());
        assert!(dir.join("task-2.log").exists());
        assert!(dir.join("result.json").exists());
        assert_eq!(std::fs::read_to_string(dir.join("DONE")).unwrap().trim(), "completed");

        // Registry bookkeeping.
        let stored = get_delegation(&record.id).unwrap();
        assert_eq!(stored.status, STATUS_COMPLETED);
        assert!(stored.finished_ms.is_some());
        let from_disk = read_result(&home, &record.id).unwrap();
        assert_eq!(from_disk["subagents"], 2);
    }

    #[tokio::test]
    async fn failures_are_reported_and_status_failed_when_all_fail() {
        let home = unique_home("fail");
        let runner: Arc<dyn SubAgentRunner> = Arc::new(FakeRunner {
            delay_ms: 1,
            fail: true,
            runs: AtomicUsize::new(0),
        });
        let record = dispatch_background_delegation(
            runner,
            vec![("x".to_string(), String::new())],
            "session-fail".to_string(),
            home.clone(),
            1,
        )
        .unwrap();
        let completions = wait_for_completion("session-fail", 5000).await;
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].result["status"], "failed");
        assert_eq!(completions[0].result["failed"], 1);
        assert!(completions[0].message.contains("boom"));
        assert_eq!(get_delegation(&record.id).unwrap().status, STATUS_FAILED);
    }

    #[tokio::test]
    async fn drain_respects_session_ownership() {
        let home = unique_home("ownership");
        let runner: Arc<dyn SubAgentRunner> = Arc::new(FakeRunner {
            delay_ms: 1,
            fail: false,
            runs: AtomicUsize::new(0),
        });
        dispatch_background_delegation(
            runner,
            vec![("only for alice".to_string(), String::new())],
            "alice".to_string(),
            home,
            1,
        )
        .unwrap();
        // Bob drains first: nothing for him, Alice's completion stays queued.
        assert!(drain_completions("bob").is_empty());
        let completions = wait_for_completion("alice", 5000).await;
        assert_eq!(completions.len(), 1);
        // Second drain is empty (claimed exactly once).
        assert!(drain_completions("alice").is_empty());
        // Empty session key never claims anything.
        assert!(drain_completions("").is_empty());
    }

    #[tokio::test]
    async fn concurrency_is_bounded() {
        let home = unique_home("bounded");
        let runner = Arc::new(FakeRunner {
            delay_ms: 60,
            fail: false,
            runs: AtomicUsize::new(0),
        });
        let runner_dyn: Arc<dyn SubAgentRunner> = runner.clone();
        let record = dispatch_background_delegation(
            runner_dyn,
            (0..4).map(|i| (format!("task {i}"), String::new())).collect(),
            "session-bounded".to_string(),
            home,
            2,
        )
        .unwrap();
        // Shortly after dispatch at most 2 children can have started.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(runner.runs.load(Ordering::SeqCst) <= 2);
        let completions = wait_for_completion("session-bounded", 5000).await;
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].result["subagents"], 4);
        assert!(get_delegation(&record.id).is_some());
    }

    #[test]
    fn report_format() {
        let consolidated = json!({
            "delegation_id": "abc",
            "status": "completed",
            "subagents": 2,
            "failed": 1,
            "results": [
                {"task": "t1", "status": "completed", "result": "done 1"},
                {"task": "t2", "status": "error", "error": "bad"},
            ],
        });
        let report = format_consolidated_report("abc", &consolidated);
        assert!(report.contains("Background delegation abc finished: 2 subagent(s), 1 failed."));
        assert!(report.contains("1. [completed] t1"));
        assert!(report.contains("done 1"));
        assert!(report.contains("2. [error] t2"));
        assert!(report.contains("error: bad"));
    }
}
