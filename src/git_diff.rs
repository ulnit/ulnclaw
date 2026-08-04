//! Working-tree git diff collection — port of hermes' `tools/working_diff.py`.
//!
//! Answers "what changed here?" with three modes:
//! - `working` (default): unstaged changes plus untracked files — what
//!   you'd lose with `git checkout . && git clean -fd`;
//! - `staged`: changes already staged for commit (`git diff --cached`);
//! - `all`: everything since HEAD (staged + unstaged) plus untracked files.
//!
//! Untracked files are folded in via `git diff --no-index /dev/null <file>`
//! so brand-new files show up as additions instead of being silently
//! invisible. This is the *git* working diff; the REPL `/diff` command
//! remains checkpoint-based.

use crate::error::{AgentError, Result};
use std::path::Path;
use std::time::{Duration, Instant};

const GIT_TIMEOUT: Duration = Duration::from_secs(15);
const GIT_TIMEOUT_LONG: Duration = Duration::from_secs(30);
const MAX_UNTRACKED_FILES: usize = 50;

/// Diff collection mode (hermes `VALID_MODES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    /// Unstaged changes (+ untracked files).
    Working,
    /// Staged changes only.
    Staged,
    /// Everything since HEAD (+ untracked files).
    All,
}

impl DiffMode {
    pub fn parse(raw: &str) -> Option<DiffMode> {
        match raw.trim().to_lowercase().as_str() {
            "working" => Some(DiffMode::Working),
            "staged" => Some(DiffMode::Staged),
            "all" => Some(DiffMode::All),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DiffMode::Working => "working",
            DiffMode::Staged => "staged",
            DiffMode::All => "all",
        }
    }

    fn base_args(&self) -> Vec<&'static str> {
        match self {
            DiffMode::Working => vec!["diff"],
            DiffMode::Staged => vec!["diff", "--cached"],
            DiffMode::All => vec!["diff", "HEAD"],
        }
    }
}

/// Collected diff (hermes result shape: stat/diff/untracked/empty).
#[derive(Debug, Clone, Default)]
pub struct WorkingDiff {
    /// `--stat` summary.
    pub stat: String,
    /// Full diff text (untracked files folded in for working/all).
    pub diff: String,
    /// Untracked file paths (working/all modes without pathspec).
    pub untracked: Vec<String>,
    /// True when there is nothing to show.
    pub empty: bool,
}

/// Run git with a polling timeout. Returns `(exit_code, stdout)`; never
/// treats a non-zero exit as an error (git diff --no-index exits 1 when
/// files differ — the success path).
fn run_git(args: &[&str], cwd: &Path, timeout: Duration) -> Result<(i32, String)> {
    let mut child = std::process::Command::new("git")
        .arg("-c")
        .arg("core.quotePath=false")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| AgentError::Tool(format!("git is not installed or not on PATH: {}", e)))?;
    let start = Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|e| AgentError::Tool(format!("git failed: {}", e)))?
        {
            Some(status) => {
                let mut output = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    use std::io::Read;
                    stdout.read_to_string(&mut output).ok();
                }
                return Ok((status.code().unwrap_or(-1), output));
            }
            None => {
                if start.elapsed() > timeout {
                    child.kill().ok();
                    return Err(AgentError::Tool("git diff timed out".to_string()));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn untracked_files(cwd: &Path) -> Vec<String> {
    let (code, out) = run_git(&["ls-files", "--others", "--exclude-standard"], cwd, GIT_TIMEOUT)
        .unwrap_or((-1, String::new()));
    if code != 0 {
        return Vec::new();
    }
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Render untracked files as new-file diffs via `git diff --no-index`.
fn untracked_diff(cwd: &Path, files: &[String]) -> String {
    let mut chunks: Vec<String> = Vec::new();
    for rel in files.iter().take(MAX_UNTRACKED_FILES) {
        let (_, out) = run_git(
            &["diff", "--no-index", "--", "/dev/null", rel],
            cwd,
            GIT_TIMEOUT,
        )
        .unwrap_or((-1, String::new()));
        let trimmed = out.trim_end_matches('\n');
        if !trimmed.trim().is_empty() {
            chunks.push(trimmed.to_string());
        }
    }
    if files.len() > MAX_UNTRACKED_FILES {
        chunks.push(format!(
            "... ({} more untracked files not shown)",
            files.len() - MAX_UNTRACKED_FILES
        ));
    }
    chunks.join("\n")
}

/// Collect a git diff of the working directory at `cwd` (hermes
/// `collect_working_diff`). `paths` optionally restricts the diff to
/// specific pathspecs (passed through to git verbatim).
pub fn collect_working_diff(
    cwd: &Path,
    mode: DiffMode,
    paths: &[String],
) -> Result<WorkingDiff> {
    // Repo probe.
    let (code, _) = run_git(&["rev-parse", "--is-inside-work-tree"], cwd, Duration::from_secs(5))?;
    if code != 0 {
        return Err(AgentError::Tool("Not a git repository.".to_string()));
    }

    let mut args: Vec<&str> = mode.base_args();
    args.push("--stat");
    let mut pathspec: Vec<&str> = Vec::new();
    if !paths.is_empty() {
        pathspec.push("--");
        for path in paths {
            pathspec.push(path.as_str());
        }
    }
    let stat_args: Vec<&str> = args.iter().chain(pathspec.iter()).copied().collect();
    let (_, stat_out) = run_git(&stat_args, cwd, GIT_TIMEOUT)?;

    let args: Vec<&str> = mode.base_args();
    let diff_args: Vec<&str> = args.iter().chain(pathspec.iter()).copied().collect();
    let (_, diff_out) = run_git(&diff_args, cwd, GIT_TIMEOUT_LONG)?;

    let mut untracked: Vec<String> = Vec::new();
    let mut untracked_text = String::new();
    if matches!(mode, DiffMode::Working | DiffMode::All) && paths.is_empty() {
        untracked = untracked_files(cwd);
        if !untracked.is_empty() {
            untracked_text = untracked_diff(cwd, &untracked);
        }
    }

    let stat = stat_out.trim().to_string();
    let mut diff = diff_out.trim().to_string();
    if !untracked_text.is_empty() {
        diff = format!("{}\n{}", diff, untracked_text);
        diff = diff.trim().to_string();
    }
    let empty = stat.is_empty() && diff.is_empty() && untracked.is_empty();
    Ok(WorkingDiff {
        stat,
        diff,
        untracked,
        empty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git runs");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ulnclaw-gitdiff-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        git(&["init", "-q"], &dir);
        git(&["config", "user.email", "test@ulnclaw.local"], &dir);
        git(&["config", "user.name", "ulnclaw-test"], &dir);
        git(&["config", "commit.gpgsign", "false"], &dir);
        std::fs::write(dir.join("tracked.txt"), "line one\n").unwrap();
        git(&["add", "tracked.txt"], &dir);
        git(&["commit", "-q", "-m", "initial"], &dir);
        dir
    }

    #[test]
    fn mode_parsing() {
        assert_eq!(DiffMode::parse("working"), Some(DiffMode::Working));
        assert_eq!(DiffMode::parse("STAGED"), Some(DiffMode::Staged));
        assert_eq!(DiffMode::parse("all"), Some(DiffMode::All));
        assert_eq!(DiffMode::parse("bogus"), None);
    }

    #[test]
    fn clean_repo_is_empty() {
        let dir = temp_repo("clean");
        let result = collect_working_diff(&dir, DiffMode::Working, &[]).unwrap();
        assert!(result.empty);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn working_mode_shows_modifications_and_untracked() {
        let dir = temp_repo("working");
        std::fs::write(dir.join("tracked.txt"), "line one\nline two\n").unwrap();
        std::fs::write(dir.join("fresh.txt"), "brand new file\n").unwrap();
        let result = collect_working_diff(&dir, DiffMode::Working, &[]).unwrap();
        assert!(!result.empty);
        assert!(result.stat.contains("tracked.txt"), "stat: {}", result.stat);
        assert!(result.diff.contains("+line two"));
        assert_eq!(result.untracked, vec!["fresh.txt".to_string()]);
        // Untracked file folded in as an addition.
        assert!(result.diff.contains("brand new file"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn staged_mode_ignores_unstaged_and_untracked() {
        let dir = temp_repo("staged");
        std::fs::write(dir.join("tracked.txt"), "staged change\n").unwrap();
        git(&["add", "tracked.txt"], &dir);
        std::fs::write(dir.join("tracked.txt"), "staged change\nunstaged too\n").unwrap();
        std::fs::write(dir.join("untracked.txt"), "not shown\n").unwrap();
        let result = collect_working_diff(&dir, DiffMode::Staged, &[]).unwrap();
        assert!(result.diff.contains("+staged change"));
        assert!(!result.diff.contains("unstaged too"));
        assert!(result.untracked.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn all_mode_covers_everything() {
        let dir = temp_repo("all");
        std::fs::write(dir.join("tracked.txt"), "modified again\n").unwrap();
        std::fs::write(dir.join("new-file.txt"), "added\n").unwrap();
        let result = collect_working_diff(&dir, DiffMode::All, &[]).unwrap();
        assert!(result.diff.contains("+modified again"));
        assert!(result.diff.contains("new-file.txt"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn not_a_repository_errors() {
        let dir = std::env::temp_dir().join(format!(
            "ulnclaw-gitdiff-norepo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let error = collect_working_diff(&dir, DiffMode::Working, &[])
            .err()
            .unwrap();
        assert!(error.to_string().contains("Not a git repository"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
