//! Skill curator — port of the local (non-LLM) half of hermes
//! `hermes_cli/curator.py` (v2026.8.3).
//!
//! The curator keeps the skill library tidy: pin/unpin, manual
//! archive/restore, usage telemetry reporting, unmanaged-skill adoption,
//! and idle pruning (bulk-archive of unpinned agent-created skills unused
//! for N days). The hermes automatic background run with LLM-driven
//! consolidation stays desktop-side; ulnclaw exposes the same lifecycle
//! verbs as explicit CLI actions.

use std::path::Path;

use crate::skill_usage::{self, STATE_ARCHIVED};

/// Days since the skill's last activity (view / use / patch) — hermes
/// `_idle_days`.
///
/// Falls back to `created_at` so a skill that was authored but never used
/// can still be pruned — otherwise never-touched skills would be immortal.
/// Returns `None` only when both fields are missing or unparseable.
pub fn idle_days(record: &serde_json::Value) -> Option<u64> {
    let raw = record
        .get("last_activity_at")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| record.get("created_at").and_then(|v| v.as_str()))?;
    let dt = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    let now = chrono::Utc::now();
    let days = (now - dt.with_timezone(&chrono::Utc)).num_days();
    Some(days.max(0) as u64)
}

/// Bulk-archive candidates: unpinned, non-archived, agent-created skills
/// idle for at least `days` (hermes `_cmd_prune` candidate selection),
/// idlest first.
pub fn prune_candidates(home: &Path, days: u64) -> Vec<(String, u64)> {
    let mut candidates: Vec<(String, u64)> = Vec::new();
    for row in skill_usage::usage_report(home) {
        if row.provenance != "agent" {
            continue;
        }
        if row.pinned || row.state == STATE_ARCHIVED {
            continue;
        }
        // Report rows carry the derived last_activity_at (hermes
        // curated_report feeds _idle_days the same shape).
        let mut record = row.record.clone();
        if let Some(ts) = &row.last_activity_at {
            record["last_activity_at"] = serde_json::json!(ts);
        }
        let Some(idle) = idle_days(&record) else {
            continue;
        };
        if idle < days {
            continue;
        }
        candidates.push((row.name, idle));
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    candidates
}

/// Human "5s/3m/4h/12d ago" rendering of an ISO timestamp (hermes
/// `_fmt_ts`).
pub fn fmt_ts(ts: Option<&str>) -> String {
    let Some(raw) = ts.filter(|s| !s.is_empty()) else {
        return "never".to_string();
    };
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return raw.to_string();
    };
    let secs = (chrono::Utc::now() - dt.with_timezone(&chrono::Utc)).num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// Status summary rows (hermes `_cmd_status`, scoped to local telemetry —
/// the background-run state lives desktop-side).
pub fn status_summary(home: &Path) -> Vec<(String, usize)> {
    let rows = skill_usage::usage_report(home);
    let archived = skill_usage::list_archived_skill_names(home).len();
    let unmanaged = skill_usage::list_unmanaged_skill_names(home).len();
    let mut states: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut pinned = 0usize;
    let mut agent_created = 0usize;
    let mut used = 0usize;
    for row in &rows {
        *states.entry(row.state.clone()).or_insert(0) += 1;
        if row.pinned {
            pinned += 1;
        }
        if row.provenance == "agent" {
            agent_created += 1;
        }
        if row.activity_count > 0 {
            used += 1;
        }
    }
    let mut out = vec![
        ("skills on disk".to_string(), rows.len()),
        ("agent-created".to_string(), agent_created),
        ("with activity".to_string(), used),
        ("pinned".to_string(), pinned),
        ("unmanaged (no provenance)".to_string(), unmanaged),
        ("archived (recoverable)".to_string(), archived),
    ];
    for (state, count) in states {
        out.push((format!("state: {}", state), count));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    static HOME_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_home() -> std::path::PathBuf {
        let n = HOME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ulnclaw-curator-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        dir
    }

    fn make_skill(home: &Path, name: &str) {
        let dir = home.join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {}\ndescription: t\n---\nbody\n", name),
        )
        .unwrap();
    }

    fn iso_days_ago(days: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    #[test]
    fn idle_days_computation() {
        let rec = json!({"last_activity_at": iso_days_ago(10), "created_at": iso_days_ago(50)});
        assert_eq!(idle_days(&rec), Some(10));
        // Falls back to created_at when there is no activity.
        let rec = json!({"created_at": iso_days_ago(30)});
        assert_eq!(idle_days(&rec), Some(30));
        // Unparseable → None.
        let rec = json!({"last_activity_at": "garbage"});
        assert_eq!(idle_days(&rec), None);
    }

    #[test]
    fn prune_selection_rules() {
        let home = temp_home();
        make_skill(&home, "idle-agent");
        make_skill(&home, "fresh-agent");
        make_skill(&home, "idle-user");
        make_skill(&home, "pinned-agent");
        let mut data = std::collections::BTreeMap::new();
        data.insert(
            "idle-agent".to_string(),
            json!({"created_by": "agent", "state": "active", "pinned": false, "created_at": iso_days_ago(120)}),
        );
        data.insert(
            "fresh-agent".to_string(),
            json!({"created_by": "agent", "state": "active", "pinned": false, "last_used_at": iso_days_ago(2), "created_at": iso_days_ago(120)}),
        );
        data.insert(
            "idle-user".to_string(),
            json!({"created_by": null, "state": "active", "pinned": false, "created_at": iso_days_ago(200)}),
        );
        data.insert(
            "pinned-agent".to_string(),
            json!({"created_by": "agent", "state": "active", "pinned": true, "created_at": iso_days_ago(300)}),
        );
        skill_usage::save_usage(&home, &data);

        let candidates = prune_candidates(&home, 90);
        let names: Vec<&str> = candidates.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["idle-agent"]);
        let (_, idle) = &candidates[0];
        assert!(*idle >= 90);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn fmt_ts_shapes() {
        assert_eq!(fmt_ts(None), "never");
        assert_eq!(fmt_ts(Some("")), "never");
        assert!(fmt_ts(Some(&iso_days_ago(3))).contains("d ago"));
        assert_eq!(fmt_ts(Some("not-a-date")), "not-a-date");
    }

    #[test]
    fn status_and_unmanaged_reports() {
        let home = temp_home();
        make_skill(&home, "managed");
        make_skill(&home, "stray");
        skill_usage::mark_agent_created(&home, "managed");
        skill_usage::bump_use(&home, "managed");

        let status = status_summary(&home);
        let get = |label: &str| {
            status
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, n)| *n)
                .unwrap_or(999)
        };
        assert_eq!(get("skills on disk"), 2);
        assert_eq!(get("agent-created"), 1);
        assert_eq!(get("with activity"), 1);
        assert_eq!(get("unmanaged (no provenance)"), 1);

        let unmanaged = skill_usage::list_unmanaged_skill_names(&home);
        assert_eq!(unmanaged, vec!["stray".to_string()]);

        // Adopt stamps provenance and clears the unmanaged list.
        let (ok, _) = skill_usage::adopt_skill(&home, "stray");
        assert!(ok);
        assert!(skill_usage::list_unmanaged_skill_names(&home).is_empty());
        // Adopting a missing skill fails cleanly.
        assert!(!skill_usage::adopt_skill(&home, "ghost").0);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn usage_report_rows() {
        let home = temp_home();
        make_skill(&home, "demo");
        skill_usage::bump_view(&home, "demo");
        skill_usage::bump_view(&home, "demo");
        skill_usage::bump_patch(&home, "demo");
        let rows = skill_usage::usage_report(&home);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "demo");
        assert_eq!(row.view_count, 2);
        assert_eq!(row.patch_count, 1);
        assert_eq!(row.activity_count, 3);
        assert_eq!(row.provenance, "user");
        assert!(row.last_activity_at.is_some());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn archived_listing() {
        let home = temp_home();
        make_skill(&home, "demo");
        assert!(skill_usage::list_archived_skill_names(&home).is_empty());
        let (ok, _) = skill_usage::archive_skill(&home, "demo");
        assert!(ok);
        assert_eq!(
            skill_usage::list_archived_skill_names(&home),
            vec!["demo".to_string()]
        );
        std::fs::remove_dir_all(&home).unwrap();
    }
}
