//! Kanban diagnostics — structured, actionable distress signals for tasks
//! (port of hermes `hermes_cli/kanban_diagnostics.py`, v2026.8.3).
//!
//! A [`Diagnostic`] is a machine-readable description of something wrong
//! with a task: a hallucinated card id, a spawn crash-loop, a task stuck
//! blocked for too long, etc. Rules run over (task, events, comments) and
//! are stateless + read-only — callers compute diagnostics on demand.

use serde::Serialize;

use crate::config::UlncLawConfig;
use crate::kanban::{Comment, KanbanStore, Task, TaskEvent};

/// Severity ladder, lowest → highest (hermes `SEVERITY_ORDER`).
pub const SEVERITY_ORDER: [&str; 3] = ["warning", "error", "critical"];

/// True when `severity` ranks at or above `threshold` (hermes
/// `severity_at_or_above`).
pub fn severity_at_or_above(severity: &str, threshold: Option<&str>) -> bool {
    let Some(threshold) = threshold else {
        return true;
    };
    let rank = |s: &str| SEVERITY_ORDER.iter().position(|x| *x == s).unwrap_or(0);
    rank(severity) >= rank(threshold)
}

/// One suggested remediation (hermes `DiagnosticAction`).
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticAction {
    pub kind: String,
    pub label: String,
    pub hint: String,
}

/// One distress signal (hermes `Diagnostic`).
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub detail: String,
    pub actions: Vec<DiagnosticAction>,
}

// Default thresholds (hermes DEFAULT_CONFIG).
const FAILURE_THRESHOLD: usize = 2;
const CRASH_THRESHOLD: usize = 2;
const BLOCKED_STALE_HOURS: i64 = 24;
const STRANDED_THRESHOLD_SECS: i64 = 30 * 60;
const BLOCK_CYCLE_WINDOW_SECS: i64 = 24 * 3600;

fn generic_recovery_actions(task: &Task) -> Vec<DiagnosticAction> {
    let mut actions = vec![DiagnosticAction {
        kind: "reassign".into(),
        label: "Reassign task".into(),
        hint: format!("ulnclaw kanban assign {} <profile>", task.id),
    }];
    if task.status == "blocked" {
        actions.push(DiagnosticAction {
            kind: "unblock".into(),
            label: "Unblock task".into(),
            hint: format!("ulnclaw kanban unblock {}", task.id),
        });
    }
    actions
}

/// `t_<hex>` task-id tokens inside free text (comments/bodies often cite
/// sibling cards; hallucinated ones are a classic agent failure mode).
fn extract_task_id_tokens(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for candidate in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-') {
        let trimmed = candidate.trim_matches(|c: char| c == '_' || c == '-');
        if trimmed.len() >= 4
            && trimmed.starts_with("t_")
            && trimmed[2..].chars().all(|c| c.is_ascii_hexdigit())
        {
            found.push(trimmed.to_string());
        }
    }
    found.sort();
    found.dedup();
    found
}

fn rule_hallucinated_cards(store: &KanbanStore, task: &Task, comments: &[Comment]) -> Vec<Diagnostic> {
    let mut phantom: Vec<String> = Vec::new();
    for comment in comments {
        for token in extract_task_id_tokens(&comment.body) {
            if token == task.id {
                continue;
            }
            let exists = store.get_task(&token).ok().flatten().is_some()
                || store.resolve_task_id(&token).ok().flatten().is_some();
            if !exists && !phantom.contains(&token) {
                phantom.push(token);
            }
        }
    }
    if phantom.is_empty() {
        return Vec::new();
    }
    vec![Diagnostic {
        kind: "hallucinated_cards".into(),
        severity: "warning".into(),
        title: format!("Comments cite {} nonexistent task id(s)", phantom.len()),
        detail: format!(
            "Comment text references task ids that do not exist on this board: {}. \
             Agents sometimes hallucinate card ids when handing off work.",
            phantom.join(", ")
        ),
        actions: vec![DiagnosticAction {
            kind: "comment".into(),
            label: "Correct the references".into(),
            hint: format!("ulnclaw kanban comment {} \"<correction>\"", task.id),
        }],
    }]
}

fn rule_prose_phantom_refs(store: &KanbanStore, task: &Task) -> Vec<Diagnostic> {
    let mut phantom: Vec<String> = Vec::new();
    for token in extract_task_id_tokens(&task.body) {
        if token == task.id {
            continue;
        }
        let exists = store.get_task(&token).ok().flatten().is_some()
            || store.resolve_task_id(&token).ok().flatten().is_some();
        if !exists {
            phantom.push(token);
        }
    }
    if phantom.is_empty() {
        return Vec::new();
    }
    vec![Diagnostic {
        kind: "prose_phantom_refs".into(),
        severity: "warning".into(),
        title: format!("Task body cites {} nonexistent task id(s)", phantom.len()),
        detail: format!(
            "The brief references task ids that do not exist: {}. A worker following \
             this body will chase phantom handoffs.",
            phantom.join(", ")
        ),
        actions: generic_recovery_actions(task),
    }]
}

fn count_events<'a>(events: impl Iterator<Item = &'a TaskEvent>, kinds: &[&str]) -> usize {
    events.filter(|event| kinds.contains(&event.kind.as_str())).count()
}

fn rule_repeated_failures(task: &Task, events: &[TaskEvent]) -> Vec<Diagnostic> {
    let failures = count_events(events.iter(), &["spawn_failed"]);
    if failures < FAILURE_THRESHOLD {
        return Vec::new();
    }
    let severity = if failures >= FAILURE_THRESHOLD * 2 {
        "critical"
    } else {
        "error"
    };
    vec![Diagnostic {
        kind: "repeated_failures".into(),
        severity: severity.into(),
        title: format!("Spawn failed {failures}x"),
        detail: format!(
            "The dispatcher failed to spawn a worker for this task {failures} times. \
             Check the worker log (<home>/kanban/worker-logs/{}.log) and the \
             ulnclaw binary on PATH.",
            task.id
        ),
        actions: vec![
            DiagnosticAction {
                kind: "cli_hint".into(),
                label: "Inspect the worker log".into(),
                hint: format!("tail ~/.ulnclaw/kanban/worker-logs/{}.log", task.id),
            },
            DiagnosticAction {
                kind: "cli_hint".into(),
                label: "Retry via unblock".into(),
                hint: format!("ulnclaw kanban unblock {}", task.id),
            },
        ],
    }]
}

fn rule_repeated_crashes(task: &Task, events: &[TaskEvent]) -> Vec<Diagnostic> {
    let crashes = count_events(events.iter(), &["released", "timed_out"]);
    if crashes < CRASH_THRESHOLD {
        return Vec::new();
    }
    let severity = if crashes >= CRASH_THRESHOLD * 2 {
        "critical"
    } else {
        "error"
    };
    vec![Diagnostic {
        kind: "repeated_crashes".into(),
        severity: severity.into(),
        title: format!("Worker died {crashes}x"),
        detail: format!(
            "Workers for this task keep ending without completing it ({crashes} \
             reclaim/timeout events). The worker prompt may be unrunnable, or the \
             runtime cap may be too tight.",
        ),
        actions: generic_recovery_actions(task),
    }]
}

fn rule_stuck_in_blocked(task: &Task, events: &[TaskEvent], now: i64) -> Vec<Diagnostic> {
    if task.status != "blocked" {
        return Vec::new();
    }
    let blocked_since = events
        .iter()
        .rev()
        .find(|event| event.kind == "blocked" || event.kind == "spawn_failed")
        .map(|event| event.created_at)
        .unwrap_or(task.created_at);
    let age_hours = (now - blocked_since) / 3600;
    if age_hours < BLOCKED_STALE_HOURS {
        return Vec::new();
    }
    vec![Diagnostic {
        kind: "stuck_in_blocked".into(),
        severity: "warning".into(),
        title: format!("Blocked for {age_hours}h"),
        detail: format!(
            "This task has been blocked for {age_hours} hours (threshold {}h). \
             Either resolve the blocker or archive the task.",
            BLOCKED_STALE_HOURS
        ),
        actions: vec![
            DiagnosticAction {
                kind: "comment".into(),
                label: "Ask for the blocker".into(),
                hint: format!("ulnclaw kanban comment {} \"what is blocking this?\"", task.id),
            },
            DiagnosticAction {
                kind: "cli_hint".into(),
                label: "Archive if stale".into(),
                hint: format!("ulnclaw kanban archive {}", task.id),
            },
        ],
    }]
}

fn rule_block_unblock_cycling(_task: &Task, events: &[TaskEvent], now: i64) -> Vec<Diagnostic> {
    let cutoff = now - BLOCK_CYCLE_WINDOW_SECS;
    let mut flips = 0usize;
    for event in events.iter().rev() {
        if event.created_at < cutoff {
            break;
        }
        if event.kind == "blocked" || event.kind == "unblocked" {
            flips += 1;
        }
    }
    let cycles = flips / 2;
    if cycles < 2 {
        return Vec::new();
    }
    vec![Diagnostic {
        kind: "block_unblock_cycling".into(),
        severity: "warning".into(),
        title: format!("Task block→unblock cycled {cycles}x in {}h", BLOCK_CYCLE_WINDOW_SECS / 3600),
        detail: "The task keeps being blocked and unblocked without progress. \
                 Break the loop: re-scope the task, reassign it, or park it in triage."
            .to_string(),
        actions: vec![DiagnosticAction {
            kind: "cli_hint".into(),
            label: "Park in triage for re-spec".into(),
            hint: "recreate with: ulnclaw kanban create --triage \"<idea>\"".into(),
        }],
    }]
}

fn rule_stranded_in_ready(task: &Task, events: &[TaskEvent], now: i64) -> Vec<Diagnostic> {
    if task.status != "ready" {
        return Vec::new();
    }
    let ever_spawned = events.iter().any(|event| event.kind == "spawned");
    if ever_spawned {
        return Vec::new();
    }
    let age = now - task.created_at;
    if age < STRANDED_THRESHOLD_SECS {
        return Vec::new();
    }
    let age_hours = age / 3600;
    let severity = if age >= STRANDED_THRESHOLD_SECS * 6 {
        "error"
    } else {
        "warning"
    };
    vec![Diagnostic {
        kind: "stranded_in_ready".into(),
        severity: severity.into(),
        title: format!("Ready for {age_hours}h but never dispatched"),
        detail: "The task sits in ready without a worker ever spawning. The \
                 dispatcher may not be running — start `ulnclaw kanban dispatch` \
                 or enable [kanban] dispatch_in_gateway in the gateway."
            .to_string(),
        actions: vec![
            DiagnosticAction {
                kind: "reassign".into(),
                label: "Run a dispatch tick".into(),
                hint: "ulnclaw kanban dispatch --dry-run".into(),
            },
            DiagnosticAction {
                kind: "cli_hint".into(),
                label: "Enable the gateway ticker".into(),
                hint: "[kanban] dispatch_in_gateway = true".into(),
            },
        ],
    }]
}

fn rule_triage_aux_unavailable(config: &UlncLawConfig, task: &Task) -> Vec<Diagnostic> {
    if task.status != "triage" {
        return Vec::new();
    }
    // A triage task needs the auxiliary LLM for specify/decompose. When no
    // main key and no [auxiliary.triage_specifier] override exist, the task
    // will sit in triage forever.
    let aux = config.auxiliary.get(crate::kanban_triage::TASK_TRIAGE_SPECIFIER);
    let aux_configured = aux.map_or(false, |task_cfg| {
        task_cfg.resolved_api_key().is_some() || task_cfg.base_url().is_some()
    });
    if aux_configured || config.resolve_api_key().is_some() {
        return Vec::new();
    }
    vec![Diagnostic {
        kind: "triage_aux_unavailable".into(),
        severity: "warning".into(),
        title: "Triage task but no auxiliary LLM configured".into(),
        detail: "Specify/decompose route through the auxiliary model, but no \
                 API key is configured. Set OPENAI_API_KEY or \
                 [auxiliary.triage_specifier] api_key/base_url."
            .to_string(),
        actions: vec![DiagnosticAction {
            kind: "cli_hint".into(),
            label: "Configure the specifier model".into(),
            hint: "[auxiliary.triage_specifier] provider/model/api_key in config.toml".into(),
        }],
    }]
}

/// Run every rule against one task and return a severity-sorted list of
/// active diagnostics (critical first, then error, then warning; hermes
/// `compute_task_diagnostics`).
pub fn compute_task_diagnostics(
    store: &KanbanStore,
    config: &UlncLawConfig,
    task: &Task,
) -> Vec<Diagnostic> {
    let now = chrono::Utc::now().timestamp();
    let events = store.events(&task.id).unwrap_or_default();
    let comments = store.comments(&task.id).unwrap_or_default();
    let mut out: Vec<Diagnostic> = Vec::new();
    out.extend(rule_hallucinated_cards(store, task, &comments));
    out.extend(rule_prose_phantom_refs(store, task));
    out.extend(rule_repeated_failures(task, &events));
    out.extend(rule_repeated_crashes(task, &events));
    out.extend(rule_stuck_in_blocked(task, &events, now));
    out.extend(rule_block_unblock_cycling(task, &events, now));
    out.extend(rule_stranded_in_ready(task, &events, now));
    out.extend(rule_triage_aux_unavailable(config, task));
    let rank = |severity: &str| {
        SEVERITY_ORDER
            .iter()
            .position(|s| *s == severity)
            .unwrap_or(0)
    };
    out.sort_by(|a, b| rank(&b.severity).cmp(&rank(&a.severity)).then(a.kind.cmp(&b.kind)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kanban::NewTask;

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

    fn kinds(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.kind.as_str()).collect()
    }

    #[test]
    fn repeated_spawn_failures_flag_error_sorted_first() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "flaky");
        for _ in 0..2 {
            store
                .append_event(&task.id, "spawn_failed", serde_json::json!({"error": "boom"}))
                .unwrap();
        }
        let diagnostics =
            compute_task_diagnostics(&store, &UlncLawConfig::default(), &task);
        assert!(kinds(&diagnostics).contains(&"repeated_failures"));
        assert_eq!(diagnostics[0].kind, "repeated_failures");
        assert_eq!(diagnostics[0].severity, "error");
    }

    #[test]
    fn stranded_ready_task_flagged_after_threshold() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "stranded");
        store.ready_task(&task.id).unwrap();
        // Age the task past the 30-minute stranded threshold.
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE tasks SET created_at = created_at - 7200 WHERE id = ?1",
                rusqlite::params![task.id],
            )
            .unwrap();
        let task = store.get_task(&task.id).unwrap().unwrap();
        let diagnostics =
            compute_task_diagnostics(&store, &UlncLawConfig::default(), &task);
        assert!(kinds(&diagnostics).contains(&"stranded_in_ready"));
    }

    #[test]
    fn hallucinated_card_ids_in_comments_flagged() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "chatty");
        store
            .add_comment(&task.id, "worker", "handoff done, see t_deadbeef for details")
            .unwrap();
        let diagnostics =
            compute_task_diagnostics(&store, &UlncLawConfig::default(), &task);
        assert!(kinds(&diagnostics).contains(&"hallucinated_cards"));
    }

    #[test]
    fn stuck_blocked_task_flagged_after_24h() {
        let (_dir, store) = temp_store();
        let task = make_task(&store, "blocked-one");
        store.block_task(&task.id, "waiting on review").unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE task_events SET created_at = created_at - 90000 WHERE kind = 'blocked'",
                [],
            )
            .unwrap();
        let task = store.get_task(&task.id).unwrap().unwrap();
        let diagnostics =
            compute_task_diagnostics(&store, &UlncLawConfig::default(), &task);
        assert!(kinds(&diagnostics).contains(&"stuck_in_blocked"));
    }

    #[test]
    fn triage_without_aux_key_flagged() {
        let (_dir, store) = temp_store();
        let task = store
            .create_task(&NewTask {
                title: "idea".into(),
                created_by: "tester".into(),
                triage: true,
                ..Default::default()
            })
            .unwrap();
        let mut config = UlncLawConfig::default();
        // No main key, no aux override → flagged.
        let diagnostics = compute_task_diagnostics(&store, &config, &task);
        let flagged = kinds(&diagnostics).contains(&"triage_aux_unavailable");
        // The CI environment may or may not have OPENAI_API_KEY; assert the
        // rule fires only when no key is resolvable.
        let keyless = config.resolve_api_key().is_none();
        assert_eq!(flagged, keyless);
        // With an explicit aux override the rule never fires.
        let mut aux_cfg = crate::config::AuxiliaryTaskConfig::default();
        aux_cfg.api_key = Some("sk-test".into());
        config
            .auxiliary
            .insert(crate::kanban_triage::TASK_TRIAGE_SPECIFIER.into(), aux_cfg);
        let diagnostics = compute_task_diagnostics(&store, &config, &task);
        assert!(!kinds(&diagnostics).contains(&"triage_aux_unavailable"));
    }

    #[test]
    fn severity_filter_thresholds() {
        assert!(severity_at_or_above("critical", Some("warning")));
        assert!(!severity_at_or_above("warning", Some("error")));
        assert!(severity_at_or_above("warning", None));
    }
}
