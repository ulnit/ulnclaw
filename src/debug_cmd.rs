//! `ulnclaw debug report` — collect a shareable, redacted diagnostic bundle
//! (hermes `hermes_cli/debug.py` collection core port; local files instead of
//! pastebin upload).
//!
//! The bundle mirrors hermes `collect_share_bundle`: a summary report
//! (`ulnclaw dump` + per-log tails) plus each full log, every file
//! self-contained with the dump header and a redaction banner. Secrets are
//! force-redacted in the collected copy; the on-disk logs are never
//! modified.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::UlncLawConfig;

/// Banner marking redacted bundle content (hermes `_REDACTION_BANNER`).
pub const REDACTION_BANNER: &str =
    "[ulnclaw debug report: log content redacted at collection time. \
     Run with --no-redact to disable]\n";

/// Cap on captured log size per file (hermes `_MAX_LOG_BYTES`).
pub const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;

/// One captured log: summary tail + optional full text (hermes
/// `LogSnapshot`).
#[derive(Debug, Clone)]
pub struct LogSnapshot {
    pub name: String,
    pub path: Option<PathBuf>,
    pub tail_text: String,
    pub full_text: Option<String>,
}

/// Where `log_name` would live if present (hermes `_primary_log_path`).
pub fn primary_log_path(log_name: &str) -> Option<PathBuf> {
    let filename = crate::logs::LOG_FILES
        .iter()
        .find(|(name, _)| *name == log_name)
        .map(|(_, file)| *file)?;
    Some(crate::config::ulnclaw_home().join("logs").join(filename))
}

/// Find the log file for `log_name`, falling back to the `.1` rotation;
/// returns the first non-empty candidate (hermes `_resolve_log_path`).
pub fn resolve_log_path(log_name: &str) -> Option<PathBuf> {
    let primary = primary_log_path(log_name)?;
    if primary.exists() && std::fs::metadata(&primary).map(|m| m.len()).unwrap_or(0) > 0 {
        return Some(primary);
    }
    let mut rotated_name = primary
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    rotated_name.push(".1");
    let rotated = primary.with_file_name(rotated_name);
    if rotated.exists() && std::fs::metadata(&rotated).map(|m| m.len()).unwrap_or(0) > 0 {
        return Some(rotated);
    }
    None
}

/// Force-redact upload-bound log text: secret passes plus email addresses
/// (hermes `_redact_log_text`). The local log file is never modified.
pub fn redact_log_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = crate::redact::redact_sensitive_text(text, crate::redact::RedactOpts::default());
    redact_emails(&out)
}

/// Replace email addresses with `[REDACTED_EMAIL]` (hermes
/// `_EMAIL_ADDRESS_RE`; the Rust `regex` crate has no lookarounds, so the
/// local-part character class simply extends the match leftwards).
fn redact_emails(text: &str) -> String {
    let Ok(re) = regex::Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}") else {
        return text.to_string();
    };
    re.replace_all(text, "[REDACTED_EMAIL]").to_string()
}
/// Last `tail_lines` lines of `text` (hermes tail rendering).
fn tail_lines(text: &str, tail_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= tail_lines {
        return text.to_string();
    }
    lines[lines.len() - tail_lines..].join("\n")
}

/// Capture one log once and derive summary/full views from the same
/// snapshot (hermes `_capture_log_snapshot`).
pub fn capture_log_snapshot(
    log_name: &str,
    tail_line_count: usize,
    max_bytes: usize,
    redact: bool,
) -> LogSnapshot {
    let Some(path) = resolve_log_path(log_name) else {
        let tail = match primary_log_path(log_name) {
            Some(primary) if primary.exists() => "(file empty)",
            _ => "(file not found)",
        };
        return LogSnapshot {
            name: log_name.to_string(),
            path: None,
            tail_text: tail.to_string(),
            full_text: None,
        };
    };

    let Ok(raw) = std::fs::read(&path) else {
        return LogSnapshot {
            name: log_name.to_string(),
            path: Some(path),
            tail_text: "(file not readable)".to_string(),
            full_text: None,
        };
    };
    if raw.is_empty() {
        return LogSnapshot {
            name: log_name.to_string(),
            path: Some(path),
            tail_text: "(file empty)".to_string(),
            full_text: None,
        };
    };

    // Keep at most max_bytes from the END of the file (recent context is
    // what support needs); align to a line boundary.
    let window: &[u8] = if raw.len() <= max_bytes {
        &raw
    } else {
        let start = raw.len() - max_bytes;
        let aligned = raw[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|off| start + off + 1)
            .unwrap_or(start);
        &raw[aligned..]
    };
    let text = String::from_utf8_lossy(window).to_string();
    let text = if redact { redact_log_text(&text) } else { text };
    let tail = tail_lines(&text, tail_line_count);
    LogSnapshot {
        name: log_name.to_string(),
        path: Some(path),
        tail_text: tail,
        full_text: Some(text),
    }
}

/// Capture every registered log (hermes `_capture_default_log_snapshots`).
pub fn capture_default_log_snapshots(
    tail_line_count: usize,
    redact: bool,
) -> BTreeMap<String, LogSnapshot> {
    let mut snapshots = BTreeMap::new();
    for (name, _) in crate::logs::LOG_FILES {
        let snapshot = capture_log_snapshot(name, tail_line_count, MAX_LOG_BYTES, redact);
        snapshots.insert(name.to_string(), snapshot);
    }
    snapshots
}

/// Build the summary report: dump + per-log tails (hermes
/// `collect_debug_report`).
pub fn collect_debug_report(
    log_lines: usize,
    dump_text: &str,
    snapshots: &BTreeMap<String, LogSnapshot>,
) -> String {
    let mut buf = String::new();
    buf.push_str(dump_text);
    buf.push_str("\n\n");
    let errors_lines = log_lines.min(100);
    let sections: &[(&str, usize)] = &[
        ("agent", log_lines),
        ("errors", errors_lines),
        ("gateway", errors_lines),
    ];
    for (name, lines) in sections {
        if let Some(snapshot) = snapshots.get(*name) {
            buf.push_str(&format!("--- {name}.log (last {lines} lines) ---\n"));
            buf.push_str(&snapshot.tail_text);
            buf.push_str("\n\n");
        }
    }
    buf
}

/// Collect report + full logs as a label→text mapping (hermes
/// `collect_share_bundle`). Each full log is self-contained (dump header
/// prepended) and carries the redaction banner when `redact` is true.
pub fn collect_share_bundle(
    config: &UlncLawConfig,
    profile: Option<&str>,
    log_lines: usize,
    redact: bool,
) -> BTreeMap<String, String> {
    let dump_text = crate::dump::build_dump(config, profile, false);
    let snapshots = capture_default_log_snapshots(log_lines, redact);
    let mut report = collect_debug_report(log_lines, &dump_text, &snapshots);
    if redact {
        report = format!("{REDACTION_BANNER}{report}");
    }

    let mut bundle = BTreeMap::new();
    bundle.insert("report.txt".to_string(), report);
    for (name, snapshot) in &snapshots {
        if let Some(full) = &snapshot.full_text {
            let mut content = format!("{}\n\n--- full {name}.log ---\n{}", dump_text, full);
            if redact {
                content = format!("{REDACTION_BANNER}{content}");
            }
            bundle.insert(format!("{name}.log"), content);
        }
    }
    bundle
}

/// CLI entry: `ulnclaw debug report [--lines N] [--no-redact] [--output DIR]`.
/// Writes the bundle files into the output directory (default:
/// `ulnclaw-debug-<timestamp>` under the current directory).
pub fn handle_debug_command(
    config: &UlncLawConfig,
    profile: Option<&str>,
    lines: usize,
    redact: bool,
    output: Option<&str>,
) -> Result<String, String> {
    let bundle = collect_share_bundle(config, profile, lines, redact);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dir = match output {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(format!("ulnclaw-debug-{stamp}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("✗ cannot create {}: {e}", dir.display()))?;

    let mut written = Vec::new();
    for (name, content) in &bundle {
        let path = dir.join(name);
        std::fs::write(&path, content)
            .map_err(|e| format!("✗ cannot write {}: {e}", path.display()))?;
        written.push(format!("  {} ({} bytes)", name, content.len()));
    }
    let mut out = format!("Debug bundle written to {}\n", dir.display());
    out.push_str(&written.join("\n"));
    out.push('\n');
    if redact {
        out.push_str("Secrets were redacted in the copies; the on-disk logs were not modified.\n");
    } else {
        out.push_str("WARNING: --no-redact — bundle files contain raw log content.\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn with_home<F: FnOnce()>(dir: &Path, f: F) {
        let _guard = crate::models_dev::test_env_lock();
        let prev = std::env::var("ULNCLAW_HOME").ok();
        std::env::set_var("ULNCLAW_HOME", dir);
        f();
        match prev {
            Some(v) => std::env::set_var("ULNCLAW_HOME", v),
            None => std::env::remove_var("ULNCLAW_HOME"),
        }
    }

    fn seed_logs(home: &Path) {
        let logs = home.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("agent.log"),
            "line one\nOPENAI_API_KEY=sk-1234567890abcdef\ncontact bob@example.com\nline four\n",
        )
        .unwrap();
        std::fs::write(logs.join("errors.log"), "boom\n").unwrap();
        // gateway.log absent on purpose.
    }

    #[test]
    fn redact_log_text_masks_secrets_and_emails() {
        let out = redact_log_text("key=sk-1234567890abcdef mail bob@example.com");
        assert!(!out.contains("1234567890"), "{out}");
        assert!(out.contains("[REDACTED_EMAIL]"), "{out}");
        assert!(!out.contains("bob@example.com"), "{out}");
    }

    #[test]
    fn resolve_falls_back_to_rotation() {
        let dir = tempfile::tempdir().unwrap();
        with_home(dir.path(), || {
            assert_eq!(resolve_log_path("agent"), None);
            let logs = dir.path().join("logs");
            std::fs::create_dir_all(&logs).unwrap();
            // Empty primary, non-empty .1 rotation -> rotated wins.
            std::fs::write(logs.join("agent.log"), "").unwrap();
            std::fs::write(logs.join("agent.log.1"), "old line\n").unwrap();
            let resolved = resolve_log_path("agent").unwrap();
            assert!(resolved.to_string_lossy().ends_with("agent.log.1"));
        });
    }

    #[test]
    fn capture_snapshot_tails_and_redacts() {
        let dir = tempfile::tempdir().unwrap();
        with_home(dir.path(), || {
            seed_logs(dir.path());
            let snapshot = capture_log_snapshot("agent", 2, MAX_LOG_BYTES, true);
            assert!(snapshot.full_text.is_some());
            let full = snapshot.full_text.unwrap();
            assert!(!full.contains("1234567890"), "{full}");
            assert!(!full.contains("bob@example.com"), "{full}");
            // Tail keeps only the requested number of lines.
            assert_eq!(snapshot.tail_text.lines().count(), 2);

            let missing = capture_log_snapshot("gateway", 10, MAX_LOG_BYTES, true);
            assert_eq!(missing.tail_text, "(file not found)");
            assert!(missing.full_text.is_none());
        });
    }

    #[test]
    fn capture_respects_max_bytes_window() {
        let dir = tempfile::tempdir().unwrap();
        with_home(dir.path(), || {
            let logs = dir.path().join("logs");
            std::fs::create_dir_all(&logs).unwrap();
            let mut content = String::new();
            for i in 0..1000 {
                content.push_str(&format!("line {i:04}\n"));
            }
            std::fs::write(logs.join("agent.log"), &content).unwrap();
            let snapshot = capture_log_snapshot("agent", 5, 100, false);
            let full = snapshot.full_text.unwrap();
            assert!(full.len() <= 120, "{}", full.len());
            assert!(full.contains("line 0999"), "{full}");
            assert!(!full.contains("line 0000"), "{full}");
        });
    }

    #[test]
    fn bundle_contains_report_and_present_logs_only() {
        let dir = tempfile::tempdir().unwrap();
        with_home(dir.path(), || {
            seed_logs(dir.path());
            let config = UlncLawConfig::default();
            let bundle = collect_share_bundle(&config, None, 50, true);
            assert!(bundle.contains_key("report.txt"));
            assert!(bundle.contains_key("agent.log"));
            assert!(bundle.contains_key("errors.log"));
            assert!(!bundle.contains_key("gateway.log"));
            let report = &bundle["report.txt"];
            assert!(report.starts_with(REDACTION_BANNER), "{report}");
            assert!(report.contains("--- ulnclaw dump ---"), "{report}");
            assert!(
                report.contains("--- agent.log (last 50 lines) ---"),
                "{report}"
            );
            let agent = &bundle["agent.log"];
            assert!(agent.contains("--- full agent.log ---"), "");
            assert!(agent.contains(REDACTION_BANNER));
        });
    }

    #[test]
    fn no_redact_skips_banner_and_keeps_content() {
        let dir = tempfile::tempdir().unwrap();
        with_home(dir.path(), || {
            seed_logs(dir.path());
            let config = UlncLawConfig::default();
            let bundle = collect_share_bundle(&config, None, 50, false);
            let agent = &bundle["agent.log"];
            assert!(!agent.contains(REDACTION_BANNER));
            assert!(agent.contains("bob@example.com"), "raw content expected");
        });
    }

    #[test]
    fn handle_writes_bundle_files() {
        let dir = tempfile::tempdir().unwrap();
        with_home(dir.path(), || {
            seed_logs(dir.path());
            let config = UlncLawConfig::default();
            let out_dir = dir.path().join("bundle-out");
            let report =
                handle_debug_command(&config, None, 50, true, Some(out_dir.to_str().unwrap()))
                    .unwrap();
            assert!(report.contains("Debug bundle written to"), "{report}");
            assert!(out_dir.join("report.txt").exists());
            assert!(out_dir.join("agent.log").exists());
            assert!(!out_dir.join("gateway.log").exists());
        });
    }
}
