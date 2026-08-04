//! Terminal tools — port of hermes' tools/terminal_tool.py
//!
//! Tools: `terminal` (foreground/background shell execution) and `process`
//! (background session management: list/log/kill/wait/write).

use crate::error::Result;
use crate::tools::{tool, ToolAvailability, ToolContext, ToolRegistry};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Default foreground timeout (hermes TERMINAL_TIMEOUT default 180).
const DEFAULT_TIMEOUT: u64 = 180;
/// Hard cap for foreground commands (hermes FOREGROUND_MAX_TIMEOUT).
const FOREGROUND_MAX_TIMEOUT: u64 = 600;
/// Max output chars returned to the model.
const MAX_OUTPUT_CHARS: usize = 100_000;

/// Check whether a shell is available (hermes check_terminal_requirements).
pub fn check_terminal_requirements() -> ToolAvailability {
    if cfg!(windows) {
        ToolAvailability::available()
    } else if PathBuf::from("/bin/sh").exists() || PathBuf::from("/bin/bash").exists() {
        ToolAvailability::available()
    } else {
        ToolAvailability::unavailable("no /bin/sh or /bin/bash found")
    }
}

// ---------------------------------------------------------------------------
// Background process registry (hermes process registry)
// ---------------------------------------------------------------------------

struct BackgroundProcess {
    command: String,
    started_at: f64,
    output: Arc<Mutex<String>>,
    exit_code: Arc<Mutex<Option<i32>>>,
    kill: Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
}

fn process_registry() -> &'static Mutex<HashMap<String, Arc<BackgroundProcess>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<BackgroundProcess>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn shell_command(command: &str) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("/bin/bash");
        c.args(["-c", command]);
        c
    };
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

fn truncate_output(output: &str) -> String {
    let count = output.chars().count();
    if count <= MAX_OUTPUT_CHARS {
        return output.to_string();
    }
    let head: String = output.chars().take(MAX_OUTPUT_CHARS / 2).collect();
    let tail: String = output.chars().skip(count - MAX_OUTPUT_CHARS / 2).collect();
    format!(
        "{}\n\n[... {} chars truncated ...]\n\n{}",
        head,
        count - MAX_OUTPUT_CHARS,
        tail
    )
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(terminal_tool());
    registry.register(process_tool());
}

fn terminal_tool() -> crate::tools::Tool {
    let timeout_note = format!(
        "Max seconds to wait (default: {}, foreground max: {}). Returns INSTANTLY when the \
         command finishes — set high for long tasks. Foreground timeout above {}s is rejected; \
         use background=true for longer commands.",
        DEFAULT_TIMEOUT, FOREGROUND_MAX_TIMEOUT, FOREGROUND_MAX_TIMEOUT
    );
    tool("terminal")
        .description(
            "Execute shell commands in the session environment. Use for builds, tests, git, \
             package managers, and any CLI work. Prefers the session working directory; \
             `cd`-only commands change it for future calls.\n\n\
             Foreground (default): blocks until the command exits or times out.\n\
             Background (background=true): returns a session_id immediately; manage it with \
             the `process` tool. Pair with notify_on_complete=true for bounded long tasks.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The command to execute"},
                "background": {"type": "boolean", "description": "Run in the background, returning a session_id. Only servers/watchers/daemons that never exit should stay silent.", "default": false},
                "timeout": {"type": "integer", "description": timeout_note, "minimum": 1},
                "workdir": {"type": "string", "description": "Working directory for this command (absolute path). Defaults to the session working directory."},
                "pty": {"type": "boolean", "description": "Run in pseudo-terminal (PTY) mode. Not supported in this build.", "default": false},
                "notify_on_complete": {"type": "boolean", "description": "With background=true: record completion for the process tool's wait action.", "default": false}
            },
            "required": ["command"]
        }))
        .handler(|args, ctx| async move {
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if command.is_empty() {
                return Ok(json!({"success": false, "error": "terminal: 'command' is required"}));
            }
            let background = args.get("background").and_then(|v| v.as_bool()).unwrap_or(false);
            let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(DEFAULT_TIMEOUT);
            let workdir = args.get("workdir").and_then(|v| v.as_str()).map(String::from);
            let pty = args.get("pty").and_then(|v| v.as_bool()).unwrap_or(false);
            terminal_exec(ctx, command, background, timeout, workdir, pty).await
        })
        .toolset("terminal")
        .dangerous(true)
        .emoji("💻")
        .check_fn(check_terminal_requirements)
        .build()
        .expect("terminal builds")
}

async fn terminal_exec(
    ctx: Arc<ToolContext>,
    command: String,
    background: bool,
    timeout: u64,
    workdir: Option<String>,
    pty: bool,
) -> Result<serde_json::Value> {
    if pty {
        return Ok(json!({
            "success": false,
            "error": "PTY mode is not supported in this build. Run the command without pty=true."
        }));
    }

    let backend = match crate::environments::resolve(&ctx.config.terminal) {
        Ok(backend) => backend,
        Err(e) => return Ok(json!({"success": false, "error": e.to_string()})),
    };

    // Bare `cd` updates the session working directory (hermes local env tracks cwd).
    let trimmed = command.trim();
    if let Some(target) = trimmed.strip_prefix("cd ") {
        let target = target.trim().trim_matches('"');
        let path = ctx.resolve_path(target);
        let remote = backend != crate::environments::TerminalBackend::Local;
        if remote || path.is_dir() {
            ctx.set_cwd(path.clone());
            return Ok(json!({
                "success": true,
                "output": String::new(),
                "cwd": path.display().to_string(),
                "note": "Session working directory updated."
            }));
        }
        return Ok(json!({"success": false, "error": format!("cd: no such directory: {}", target)}));
    }

    let cwd = match workdir {
        Some(ref dir) => {
            let path = ctx.resolve_path(dir);
            if !path.is_dir() {
                return Ok(json!({"success": false, "error": format!("workdir not found: {}", path.display())}));
            }
            path
        }
        None => ctx.cwd(),
    };

    if background {
        return spawn_background(ctx, command, cwd, &backend).await;
    }

    if timeout > FOREGROUND_MAX_TIMEOUT {
        return Ok(json!({
            "success": false,
            "error": format!(
                "Foreground timeout {}s exceeds the {}s maximum. Use background=true for longer commands.",
                timeout, FOREGROUND_MAX_TIMEOUT
            )
        }));
    }

    let started = std::time::Instant::now();
    let effective = crate::environments::wrap_command(
        &backend,
        &command,
        cwd.to_str(),
    );
    let mut cmd = shell_command(&effective);
    if backend == crate::environments::TerminalBackend::Local {
        cmd.current_dir(&cwd);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Ok(json!({"success": false, "error": format!("spawn failed: {}", e)})),
    };

    // Drain stdout/stderr in tasks so wait() doesn't deadlock.
    let stdout_task = child.stdout.take().map(|mut stream| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.ok();
            buf
        })
    });
    let stderr_task = child.stderr.take().map(|mut stream| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.ok();
            buf
        })
    });

    let result = tokio::time::timeout(Duration::from_secs(timeout.max(1)), child.wait()).await;
    let duration = started.elapsed().as_secs_f64();
    match result {
        Ok(Ok(status)) => {
            let stdout = match stdout_task {
                Some(handle) => String::from_utf8_lossy(&handle.await.unwrap_or_default()).into_owned(),
                None => String::new(),
            };
            let stderr = match stderr_task {
                Some(handle) => String::from_utf8_lossy(&handle.await.unwrap_or_default()).into_owned(),
                None => String::new(),
            };
            let combined = if stderr.trim().is_empty() {
                stdout
            } else if stdout.trim().is_empty() {
                stderr
            } else {
                format!("{}\n[stderr]\n{}", stdout, stderr)
            };
            let exit_code = status.code().unwrap_or(-1);
            let output = truncate_output(combined.trim_end());
            let mut result = json!({
                "success": status.success(),
                "exit_code": exit_code,
                "output": output,
                "duration_seconds": duration,
            });
            if !status.success() {
                // hermes terminal_tool.py: explain benign exit codes, else
                // attach one actionable failure hint (tools/hints.rs).
                if let Some(note) = crate::tools::hints::interpret_exit_code(&command, exit_code) {
                    result["exit_code_meaning"] = json!(note);
                } else if let Some(hint) = crate::tools::hints::annotate_failure(&command, exit_code, &output) {
                    result["hint"] = json!(hint);
                }
            }
            Ok(result)
        }
        Ok(Err(e)) => Ok(json!({"success": false, "error": format!("wait failed: {}", e)})),
        Err(_) => {
            child.kill().await.ok();
            Ok(json!({
                "success": false,
                "error": format!("Command timed out after {}s. Use background=true for long-running work.", timeout),
                "duration_seconds": duration,
            }))
        }
    }
}

async fn spawn_background(
    ctx: Arc<ToolContext>,
    command: String,
    cwd: PathBuf,
    backend: &crate::environments::TerminalBackend,
) -> Result<serde_json::Value> {
    let session_id = format!("bg-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let effective = crate::environments::wrap_command(backend, &command, cwd.to_str());
    let mut cmd = shell_command(&effective);
    if *backend == crate::environments::TerminalBackend::Local {
        cmd.current_dir(&cwd);
    }
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Ok(json!({"success": false, "error": format!("spawn failed: {}", e)})),
    };

    let output_buffer = Arc::new(Mutex::new(String::new()));
    let exit_code: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));

    let (stdout, stderr, wait_child) = {
        let mut child = child;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        (stdout, stderr, child)
    };

    let buffer_clone = output_buffer.clone();
    tokio::spawn(async move {
        if let Some(mut stream) = stdout {
            let buf = buffer_clone.clone();
            tokio::spawn(async move {
                let mut chunk = vec![0u8; 8192];
                loop {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut guard) = buf.lock() {
                                guard.push_str(&String::from_utf8_lossy(&chunk[..n]));
                                if guard.len() > MAX_OUTPUT_CHARS * 2 {
                                    let drain = guard.len() - MAX_OUTPUT_CHARS;
                                    guard.drain(..drain);
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    let buffer_clone = output_buffer.clone();
    tokio::spawn(async move {
        if let Some(mut stream) = stderr {
            let mut chunk = vec![0u8; 8192];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut guard) = buffer_clone.lock() {
                            guard.push_str(&String::from_utf8_lossy(&chunk[..n]));
                        }
                    }
                }
            }
        }
    });

    let exit_clone = exit_code.clone();
    let kill_mutex = Arc::new(tokio::sync::Mutex::new(Some(wait_child)));
    let kill_handle = kill_mutex.clone();
    tokio::spawn(async move {
        let status = {
            let mut guard = kill_handle.lock().await;
            match guard.as_mut() {
                Some(child) => child.wait().await,
                None => return,
            }
        };
        if let Ok(code) = status.map(|s| s.code().unwrap_or(-1)) {
            if let Ok(mut guard) = exit_clone.lock() {
                *guard = Some(code);
            }
        }
    });

    let record = Arc::new(BackgroundProcess {
        command: command.clone(),
        started_at: now_secs(),
        output: output_buffer,
        exit_code,
        kill: kill_mutex,
    });
    process_registry()
        .lock()
        .map(|mut reg| reg.insert(session_id.clone(), record))
        .ok();

    let _ = &ctx;
    Ok(json!({
        "success": true,
        "session_id": session_id,
        "status": "running",
        "note": "Process started in background. Use the process tool with action=log/wait/kill to manage it.",
    }))
}

fn process_tool() -> crate::tools::Tool {
    tool("process")
        .description(
            "Manage background terminal sessions started with terminal(background=true).\n\
             Actions:\n\
             - list: show all background sessions with status\n\
             - log: tail output (args: session_id, limit lines)\n\
             - wait: block until the process exits (args: session_id, timeout)\n\
             - kill: terminate the session (args: session_id)",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "log", "wait", "kill"], "description": "What to do"},
                "session_id": {"type": "string", "description": "Background session id (from terminal background=true). Required for log/wait/kill."},
                "limit": {"type": "integer", "description": "For log: number of trailing lines to return (default 100)", "default": 100},
                "timeout": {"type": "integer", "description": "For wait: max seconds to block (default 60)", "default": 60}
            },
            "required": ["action"]
        }))
        .handler(|args, _ctx| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            let session_id = args.get("session_id").and_then(|v| v.as_str()).map(String::from);
            match action {
                "list" => {
                    let reg = process_registry().lock().map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                    let mut items = Vec::new();
                    for (id, proc) in reg.iter() {
                        let running = proc.exit_code.lock().map(|g| g.is_none()).unwrap_or(false);
                        items.push(json!({
                            "session_id": id,
                            "command": proc.command,
                            "status": if running { "running" } else { "exited" },
                            "exit_code": proc.exit_code.lock().ok().and_then(|g| *g),
                            "age_seconds": (now_secs() - proc.started_at).round(),
                        }));
                    }
                    Ok(json!({"success": true, "sessions": items}))
                }
                "log" | "wait" | "kill" => {
                    let Some(session_id) = session_id else {
                        return Ok(json!({"success": false, "error": format!("process {}: session_id is required", action)}));
                    };
                    let proc = {
                        let reg = process_registry().lock().map_err(|e| crate::error::AgentError::tool(e.to_string()))?;
                        reg.get(&session_id).cloned()
                    };
                    let Some(proc) = proc else {
                        return Ok(json!({"success": false, "error": format!("Unknown session_id: {}", session_id)}));
                    };
                    match action {
                        "log" => {
                            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
                            let output = proc.output.lock().map(|g| g.clone()).unwrap_or_default();
                            let lines: Vec<&str> = output.lines().collect();
                            let tail: Vec<&str> = lines.iter().rev().take(limit).cloned().collect::<Vec<_>>().into_iter().rev().collect();
                            let running = proc.exit_code.lock().map(|g| g.is_none()).unwrap_or(false);
                            Ok(json!({
                                "success": true,
                                "session_id": session_id,
                                "status": if running { "running" } else { "exited" },
                                "exit_code": proc.exit_code.lock().ok().and_then(|g| *g),
                                "output": truncate_output(&tail.join("\n")),
                            }))
                        }
                        "wait" => {
                            let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(60);
                            let deadline = std::time::Instant::now() + Duration::from_secs(timeout.max(1));
                            loop {
                                if let Ok(guard) = proc.exit_code.lock() {
                                    if let Some(code) = *guard {
                                        let output = proc.output.lock().map(|g| g.clone()).unwrap_or_default();
                                        return Ok(json!({
                                            "success": true,
                                            "session_id": session_id,
                                            "status": "exited",
                                            "exit_code": code,
                                            "output": truncate_output(output.trim_end()),
                                        }));
                                    }
                                }
                                if std::time::Instant::now() >= deadline {
                                    return Ok(json!({
                                        "success": true,
                                        "session_id": session_id,
                                        "status": "running",
                                        "note": format!("Still running after {}s wait.", timeout),
                                    }));
                                }
                                tokio::time::sleep(Duration::from_millis(250)).await;
                            }
                        }
                        "kill" => {
                            let mut guard = proc.kill.lock().await;
                            if let Some(ref mut child) = *guard {
                                child.kill().await.ok();
                            }
                            *guard = None;
                            Ok(json!({"success": true, "session_id": session_id, "status": "killed"}))
                        }
                        _ => unreachable!(),
                    }
                }
                other => Ok(json!({"success": false, "error": format!("Unknown action: {}", other)})),
            }
        })
        .toolset("terminal")
        .emoji("⚙️")
        .check_fn(check_terminal_requirements)
        .build()
        .expect("process builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_foreground_echo() {
        let ctx = Arc::new(ToolContext::new());
        let result = terminal_exec(ctx, "echo hello-ulnclaw".into(), false, 10, None, false)
            .await
            .unwrap();
        assert_eq!(result["success"], json!(true));
        assert!(result["output"].as_str().unwrap().contains("hello-ulnclaw"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let ctx = Arc::new(ToolContext::new());
        let result = terminal_exec(ctx, "sleep 5".into(), false, 1, None, false)
            .await
            .unwrap();
        assert_eq!(result["success"], json!(false));
        assert!(result["error"].as_str().unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn test_cd_updates_workdir() {
        let ctx = Arc::new(ToolContext::new());
        let dir = tempfile::tempdir().unwrap();
        let result = terminal_exec(
            ctx.clone(),
            format!("cd {}", dir.path().display()),
            false,
            5,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(result["success"], json!(true));
        assert_eq!(ctx.cwd(), dir.path().canonicalize().unwrap_or(dir.path().to_path_buf()));
    }

    #[tokio::test]
    async fn test_benign_exit_code_meaning() {
        let ctx = Arc::new(ToolContext::new());
        let result = terminal_exec(
            ctx,
            "grep -q ulnclaw-no-match-zzz /dev/null".into(),
            false,
            10,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(result["exit_code"], json!(1));
        assert_eq!(result["exit_code_meaning"], json!("No matches found (not an error)"));
        assert!(result.get("hint").is_none());
    }

    #[tokio::test]
    async fn test_failure_hint_command_not_found() {
        let ctx = Arc::new(ToolContext::new());
        let result = terminal_exec(
            ctx,
            "ulnclaw-no-such-cmd-xyz".into(),
            false,
            10,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(result["exit_code"], json!(127));
        let hint = result["hint"].as_str().expect("hint present");
        assert!(hint.contains("ulnclaw-no-such-cmd-xyz"), "got: {hint}");
    }
}
