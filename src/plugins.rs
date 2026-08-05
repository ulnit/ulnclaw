//! Plugin system — Rust-native port of hermes' plugin architecture.
//!
//! Hermes loads Python plugins (`hermes_cli/plugins.py`) and bridges
//! shell scripts (`agent/shell_hooks.py`) through one `invoke_hook()`
//! aggregator. A static Rust binary can't import Python, so ulnclaw uses
//! the shell-hook wire protocol for everything:
//!
//! * **Directory plugins**: `<home>/plugins/<name>/plugin.toml` manifest
//!   declaring `hooks` (scripts at `hooks/<event>`) and `[[tools]]`
//!   (scripts invoked with `{"tool", "arguments"}` on stdin, registered
//!   as `plugin__<plugin>__<tool>`).
//! * **Config shell hooks**: `[hooks]` block in config.toml maps event
//!   names to command lines (shlex-split, no shell — hermes policy).
//!
//! Wire protocol (hermes shell_hooks): stdin JSON
//! `{"hook_event_name", "tool_name", "tool_input", "session_id", "cwd",
//! "extra"}`; stdout JSON `{"decision"|"action": "block", ...}` to block
//! a `pre_tool_call`, `{"text": ...}` to transform output, anything else
//! is an observer no-op. First-use consent for config hooks lives in
//! `<home>/shell-hooks-allowlist.json` (hermes `shell-hooks-allowlist`),
//! bypassed by `[hooks] auto_accept = true` or `ULNCLAW_ACCEPT_HOOKS=1`.

use crate::tools::{tool, ToolAvailability, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;

/// Hook names the agent core fires (hermes `VALID_HOOKS`).
pub const VALID_HOOKS: &[&str] = &[
    "pre_tool_call",
    "post_tool_call",
    "transform_terminal_output",
    "transform_tool_result",
    "transform_llm_output",
    "pre_llm_call",
    "post_llm_call",
    "pre_verify",
    "pre_api_request",
    "post_api_request",
    "api_request_error",
    "on_session_start",
    "on_session_end",
    "on_session_finalize",
    "on_session_reset",
    "subagent_start",
    "subagent_stop",
    "pre_gateway_dispatch",
    "pre_approval_request",
    "post_approval_response",
    "kanban_task_claimed",
    "kanban_task_completed",
    "kanban_task_blocked",
];

/// Per-hook subprocess wall-clock cap.
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// `[plugins]` config block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// Plugins never loaded (hermes deny-list semantics).
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// `[hooks]` config block: event → command lines (plus `auto_accept`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Accept new hook commands without the first-use consent prompt.
    #[serde(default)]
    pub auto_accept: bool,
    /// Event name → command lines (every other key).
    #[serde(flatten)]
    pub events: HashMap<String, Vec<String>>,
}

/// One `[[tools]]` entry in a plugin.toml manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Executable, relative to the plugin directory (or absolute).
    pub command: String,
    /// Optional JSON schema for the arguments (inline TOML table).
    #[serde(default)]
    pub parameters: Option<toml::Value>,
}

/// Parsed `plugin.toml` manifest (hermes `plugin.yaml` PluginManifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// Hook events this plugin handles via `hooks/<event>` scripts.
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub tools: Vec<PluginToolSpec>,
}

/// A discovered plugin plus its directory.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
    /// On the config deny-list: listed by `plugins list` but contributes
    /// no hooks or tools.
    pub disabled: bool,
}

/// One registered hook callback.
#[derive(Debug, Clone)]
struct HookCallback {
    event: String,
    /// Absolute script path (directory plugin) or command line (config).
    command: String,
    /// True for config shell hooks (consent-tracked); plugin scripts are
    /// trusted by installation (hermes user-source semantics).
    needs_consent: bool,
}

struct PluginRuntime {
    plugins: Vec<LoadedPlugin>,
    hooks: Vec<HookCallback>,
    warnings: Vec<String>,
}

static RUNTIME: OnceCell<PluginRuntime> = OnceCell::const_new();

/// Initialize the plugin runtime (idempotent). Returns discovery warnings.
pub async fn init(home: &Path, config: &crate::config::UlncLawConfig) -> Vec<String> {
    let runtime = RUNTIME
        .get_or_init(|| async { build_runtime(home, config).await })
        .await;
    runtime.warnings.clone()
}

fn plugins_dir(home: &Path) -> PathBuf {
    home.join("plugins")
}

fn allowlist_path(home: &Path) -> PathBuf {
    home.join("shell-hooks-allowlist.json")
}

fn load_allowlist(home: &Path) -> Vec<String> {
    std::fs::read_to_string(allowlist_path(home))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("accepted").and_then(|a| a.as_array()).cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn save_allowlist(home: &Path, accepted: &[String]) {
    let value = json!({"accepted": accepted});
    if let Ok(text) = serde_json::to_string_pretty(&value) {
        std::fs::write(allowlist_path(home), text).ok();
    }
}

/// Shlex-style split (hermes `shlex.split`, shell=False): whitespace
/// separates tokens; single/double quotes group; backslash escapes.
pub fn split_command(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_token = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && !in_single {
            if let Some(&next) = chars.peek() {
                current.push(next);
                chars.next();
                has_token = true;
            }
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            has_token = true;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            has_token = true;
            continue;
        }
        if c.is_whitespace() && !in_single && !in_double {
            if has_token {
                out.push(std::mem::take(&mut current));
                has_token = false;
            }
            continue;
        }
        current.push(c);
        has_token = true;
    }
    if has_token {
        out.push(current);
    }
    out
}

async fn build_runtime(home: &Path, config: &crate::config::UlncLawConfig) -> PluginRuntime {
    let mut warnings = Vec::new();
    let mut plugins = Vec::new();
    let mut hooks: Vec<HookCallback> = Vec::new();

    // 1. Directory plugins (hermes user-source discovery).
    let dir = plugins_dir(home);
    if dir.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        entries.sort();
        for entry in entries {
            let manifest_path = entry.join("plugin.toml");
            if !manifest_path.is_file() {
                warnings.push(format!(
                    "plugins: {} skipped (no plugin.toml)",
                    entry.display()
                ));
                continue;
            }
            match std::fs::read_to_string(&manifest_path)
                .map_err(|e| e.to_string())
                .and_then(|text| toml::from_str::<PluginManifest>(&text).map_err(|e| e.to_string()))
            {
                Ok(manifest) => {
                    if manifest.name.trim().is_empty() {
                        warnings.push(format!(
                            "plugins: {} skipped (manifest has no name)",
                            entry.display()
                        ));
                        continue;
                    }
                    let disabled = config.plugins.disabled.iter().any(|d| d == &manifest.name);
                    if disabled {
                        plugins.push(LoadedPlugin { manifest, dir: entry, disabled: true });
                        continue;
                    }
                    for hook in &manifest.hooks {
                        if !VALID_HOOKS.contains(&hook.as_str()) {
                            warnings.push(format!(
                                "plugins: {} declares unknown hook {hook:?}",
                                manifest.name
                            ));
                            continue;
                        }
                        let script = entry.join("hooks").join(hook);
                        if script.is_file() {
                            hooks.push(HookCallback {
                                event: hook.clone(),
                                command: script.display().to_string(),
                                needs_consent: false,
                            });
                        } else {
                            warnings.push(format!(
                                "plugins: {} hook {hook:?} has no hooks/{hook} script",
                                manifest.name
                            ));
                        }
                    }
                    plugins.push(LoadedPlugin { manifest, dir: entry, disabled: false });
                }
                Err(e) => {
                    warnings.push(format!("plugins: {} manifest error: {e}", entry.display()));
                }
            }
        }
    }

    // 2. Config shell hooks with first-use consent (hermes shell_hooks).
    let auto_accept = config.hooks.auto_accept
        || std::env::var("ULNCLAW_ACCEPT_HOOKS")
            .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
    let mut allowlist = load_allowlist(home);
    let mut allowlist_changed = false;
    let mut events: Vec<(&String, &Vec<String>)> = config.hooks.events.iter().collect();
    events.sort_by(|a, b| a.0.cmp(b.0));
    for (event, commands) in events {
        if !VALID_HOOKS.contains(&event.as_str()) {
            warnings.push(format!("hooks: unknown event {event:?} in config (ignored)"));
            continue;
        }
        for command in commands {
            let key = format!("{event}\t{command}");
            if allowlist.iter().any(|a| a == &key) {
                hooks.push(HookCallback {
                    event: event.clone(),
                    command: command.clone(),
                    needs_consent: true,
                });
            } else if auto_accept {
                allowlist.push(key);
                allowlist_changed = true;
                hooks.push(HookCallback {
                    event: event.clone(),
                    command: command.clone(),
                    needs_consent: true,
                });
            } else {
                warnings.push(format!(
                    "hooks: {event:?} -> {command:?} not consented yet — run \
                     `ulnclaw plugins accept-hooks` or set [hooks] auto_accept = true"
                ));
            }
        }
    }
    if allowlist_changed {
        save_allowlist(home, &allowlist);
    }

    PluginRuntime { plugins, hooks, warnings }
}

/// Discovered plugins (empty before `init`).
pub fn loaded_plugins() -> Vec<LoadedPlugin> {
    RUNTIME
        .get()
        .map(|r| r.plugins.clone())
        .unwrap_or_default()
}

/// Registered hook callbacks for one event, plugins first (hermes:
/// plugin block decisions win ties over shell hooks).
fn callbacks_for(event: &str) -> Vec<HookCallback> {
    RUNTIME
        .get()
        .map(|r| {
            r.hooks
                .iter()
                .filter(|h| h.event == event)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Run one hook script: JSON payload on stdin, JSON response on stdout.
/// Never fatal — timeouts/parse errors yield no response (hermes policy).
async fn run_hook_callback(callback: &HookCallback, payload: &Value) -> Option<Value> {
    let argv = split_command(&callback.command);
    if argv.is_empty() {
        return None;
    }
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        // Timeout drops the wait future; kill_on_drop reaps the script.
        .kill_on_drop(true);
    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let body = serde_json::to_string(payload).unwrap_or_default();
        if stdin.write_all(body.as_bytes()).await.is_err() {
            child.kill().await.ok();
            return None;
        }
    }
    let output = tokio::time::timeout(HOOK_TIMEOUT, child.wait_with_output()).await;
    let output = match output {
        Ok(Ok(output)) if output.status.success() => output,
        _ => return None,
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed).ok()
}

/// Fire one hook across every registered callback; returns all responses
/// (hermes `invoke_hook` — aggregation is the caller's job).
pub async fn invoke_hook(event: &str, payload: Value) -> Vec<Value> {
    let callbacks = callbacks_for(event);
    if callbacks.is_empty() {
        return Vec::new();
    }
    let mut responses = Vec::new();
    for callback in callbacks {
        if let Some(response) = run_hook_callback(&callback, &payload).await {
            responses.push(response);
        }
    }
    responses
}

/// Build the hermes hook payload envelope.
pub fn hook_payload(
    event: &str,
    session_id: &str,
    cwd: &Path,
    fields: Vec<(&str, Value)>,
    extra: Value,
) -> Value {
    let mut payload = json!({
        "hook_event_name": event,
        "session_id": session_id,
        "cwd": cwd.display().to_string(),
    });
    for (key, value) in fields {
        payload[key] = value;
    }
    payload["extra"] = extra;
    payload
}

/// pre_tool_call aggregation: first block wins (hermes semantics —
/// both Claude-Code and hermes shapes accepted). Returns the reason.
pub fn block_decision(responses: &[Value]) -> Option<String> {
    for response in responses {
        let decision = response
            .get("decision")
            .or_else(|| response.get("action"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if decision.eq_ignore_ascii_case("block") {
            let reason = response
                .get("reason")
                .or_else(|| response.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("blocked by plugin hook");
            return Some(reason.to_string());
        }
    }
    None
}

/// transform_* aggregation: first non-empty text wins (hermes).
pub fn transform_text(responses: &[Value]) -> Option<String> {
    for response in responses {
        let candidate = match response {
            Value::String(s) => Some(s.clone()),
            Value::Object(_) => response
                .get("text")
                .or_else(|| response.get("output"))
                .and_then(|v| v.as_str())
                .map(String::from),
            _ => None,
        };
        if let Some(text) = candidate {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// pre_llm_call aggregation: collect injected context strings (hermes).
pub fn context_injections(responses: &[Value]) -> Vec<String> {
    responses
        .iter()
        .filter_map(|r| r.get("context").and_then(|v| v.as_str()).map(String::from))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Register every plugin tool into the registry as
/// `plugin__<plugin>__<tool>` (hermes `register_tool` namespacing).
pub fn register_plugin_tools(registry: &mut ToolRegistry) -> usize {
    let mut count = 0;
    for plugin in loaded_plugins() {
        if plugin.disabled {
            continue;
        }
        for spec in &plugin.manifest.tools {
            let qualified = format!("plugin__{}__{}", plugin.manifest.name, spec.name);
            let command_path = if Path::new(&spec.command).is_absolute() {
                PathBuf::from(&spec.command)
            } else {
                plugin.dir.join(&spec.command)
            };
            if !command_path.is_file() {
                continue;
            }
            let parameters = spec
                .parameters
                .as_ref()
                .and_then(|t| serde_json::to_value(t).ok())
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            let description = if spec.description.is_empty() {
                format!("[plugin {}] {}", plugin.manifest.name, spec.name)
            } else {
                format!("[plugin {}] {}", plugin.manifest.name, spec.description)
            };
            let toolset = format!("plugin:{}", plugin.manifest.name);
            let handler_path = command_path.clone();
            let check_path = command_path.clone();
            let panic_name = qualified.clone();
            let tool_name = spec.name.clone();
            registry.register(
                tool(qualified)
                    .description(description)
                    .parameters(parameters)
                    .handler(move |args, _ctx| {
                        let command_path = handler_path.clone();
                        let tool_name = tool_name.clone();
                        async move {
                            let payload = json!({"tool": tool_name, "arguments": args});
                            let mut cmd = tokio::process::Command::new(&command_path);
                            cmd.stdin(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::piped())
                                .stderr(std::process::Stdio::null());
                            let mut child = cmd.spawn().map_err(|e| {
                                crate::error::AgentError::Tool(format!(
                                    "plugin tool spawn failed: {e}"
                                ))
                            })?;
                            if let Some(mut stdin) = child.stdin.take() {
                                stdin
                                    .write_all(serde_json::to_string(&payload).unwrap_or_default().as_bytes())
                                    .await
                                    .ok();
                            }
                            let output = tokio::time::timeout(
                                Duration::from_secs(120),
                                child.wait_with_output(),
                            )
                            .await
                            .map_err(|_| {
                                crate::error::AgentError::Tool("plugin tool timed out".into())
                            })?
                            .map_err(|e| crate::error::AgentError::Tool(format!("plugin tool: {e}")))?;
                            let text = String::from_utf8_lossy(&output.stdout);
                            if !output.status.success() {
                                return Ok(json!({
                                    "success": false,
                                    "error": format!("plugin tool exited with {}", output.status),
                                }));
                            }
                            Ok(serde_json::from_str::<Value>(text.trim())
                                .unwrap_or_else(|_| json!({"success": true, "output": text.trim()})))
                        }
                    })
                    .toolset(toolset)
                    .emoji("🧩")
                    .check_fn(move || {
                        if check_path.is_file() {
                            ToolAvailability::available()
                        } else {
                            ToolAvailability::unavailable("plugin tool script missing")
                        }
                    })
                    .build()
                    .unwrap_or_else(|_| panic!("{panic_name} builds")),
            );
            count += 1;
        }
    }
    count
}

/// transform_llm_output convenience (hermes turn_finalizer site): run the
/// hook over the final assistant text; first non-empty replacement wins.
pub async fn transform_llm_output(session_id: &str, cwd: &Path, text: &str) -> String {
    let payload = hook_payload(
        "transform_llm_output",
        session_id,
        cwd,
        vec![("response_text", serde_json::json!(text))],
        serde_json::json!({}),
    );
    let responses = invoke_hook("transform_llm_output", payload).await;
    transform_text(&responses).unwrap_or_else(|| text.to_string())
}

/// Fire a session lifecycle hook (on_session_start / on_session_end /
/// on_session_finalize / on_session_reset) — observers, never fatal.
pub async fn fire_session_event(event: &str, session_id: &str, cwd: &Path, extra: Value) {
    let payload = hook_payload(event, session_id, cwd, vec![], extra);
    let _ = invoke_hook(event, payload).await;
}

/// Accept all pending config hooks into the allowlist (CLI helper).
pub fn accept_all_hooks(home: &Path, config: &crate::config::UlncLawConfig) -> usize {
    let mut allowlist = load_allowlist(home);
    let mut added = 0;
    for (event, commands) in &config.hooks.events {
        if !VALID_HOOKS.contains(&event.as_str()) {
            continue;
        }
        for command in commands {
            let key = format!("{event}\t{command}");
            if !allowlist.iter().any(|a| a == &key) {
                allowlist.push(key);
                added += 1;
            }
        }
    }
    if added > 0 {
        save_allowlist(home, &allowlist);
    }
    added
}

/// Read the `plugins.disabled` list from config.toml (source of truth for
/// enable/disable persistence).
fn read_disabled_list(config_path: &Path) -> Vec<String> {
    std::fs::read_to_string(config_path)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
        .and_then(|doc| doc.get("plugins").and_then(|p| p.get("disabled")).cloned())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn write_disabled_list(config_path: &Path, disabled: &[String]) -> Result<(), String> {
    // Never clobber on parse failure — refuse instead.
    let text = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Table = if text.trim().is_empty() {
        toml::Table::new()
    } else {
        toml::from_str(&text).map_err(|e| format!("config.toml parse failed: {e}"))?
    };
    let plugins = doc
        .entry("plugins")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let plugins_table = plugins.as_table_mut().ok_or("[plugins] is not a table")?;
    plugins_table.insert(
        "disabled".to_string(),
        toml::Value::Array(disabled.iter().map(|s| toml::Value::String(s.clone())).collect()),
    );
    // toml::to_string renders document form ([plugins] section), unlike
    // Value's Display which emits inline tables.
    let rendered = toml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(config_path, rendered).map_err(|e| e.to_string())
}

/// Current deny-list straight from config.toml (fresh read for `plugins
/// list` after enable/disable in the same process).
pub fn current_disabled(home: &Path) -> Vec<String> {
    read_disabled_list(&home.join("config.toml"))
}

/// Enable a plugin (remove from the deny-list) — hermes `plugins enable`.
pub fn enable_plugin(home: &Path, name: &str) -> Result<String, String> {
    let config_path = home.join("config.toml");
    let mut disabled = read_disabled_list(&config_path);
    if !disabled.iter().any(|d| d == name) {
        return Ok(format!("{name} is already enabled."));
    }
    disabled.retain(|d| d != name);
    write_disabled_list(&config_path, &disabled)?;
    Ok(format!("✓ Enabled plugin {name} (takes effect on next start)."))
}

/// Disable a plugin (add to the deny-list) — hermes `plugins disable`.
pub fn disable_plugin(home: &Path, name: &str) -> Result<String, String> {
    let config_path = home.join("config.toml");
    let mut disabled = read_disabled_list(&config_path);
    if disabled.iter().any(|d| d == name) {
        return Ok(format!("{name} is already disabled."));
    }
    disabled.push(name.to_string());
    write_disabled_list(&config_path, &disabled)?;
    Ok(format!("✓ Disabled plugin {name} (takes effect on next start)."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_command_handles_quotes_and_escapes() {
        assert_eq!(split_command("a b c"), vec!["a", "b", "c"]);
        assert_eq!(
            split_command("echo 'hello world' \"x y\""),
            vec!["echo", "hello world", "x y"]
        );
        assert_eq!(split_command("a\\ b c"), vec!["a b", "c"]);
        assert_eq!(split_command("  spaced   out  "), vec!["spaced", "out"]);
        assert!(split_command("").is_empty());
    }

    #[test]
    fn manifest_parses() {
        let manifest: PluginManifest = toml::from_str(
            r#"
            name = "demo"
            description = "demo plugin"
            version = "0.1.0"
            hooks = ["pre_tool_call", "on_session_end"]
            [[tools]]
            name = "hello"
            description = "say hi"
            command = "tools/hello"
            "#,
        )
        .unwrap();
        assert_eq!(manifest.name, "demo");
        assert_eq!(manifest.hooks.len(), 2);
        assert_eq!(manifest.tools[0].name, "hello");
    }

    #[test]
    fn block_decision_shapes() {
        let responses = vec![json!({}), json!({"decision": "block", "reason": "nope"})];
        assert_eq!(block_decision(&responses).as_deref(), Some("nope"));
        let responses = vec![json!({"action": "block", "message": "halt"})];
        assert_eq!(block_decision(&responses).as_deref(), Some("halt"));
        let responses = vec![json!({"decision": "allow"})];
        assert!(block_decision(&responses).is_none());
    }

    #[test]
    fn transform_first_nonempty_wins() {
        let responses = vec![json!({}), json!({"text": ""}), json!({"text": "hi"})];
        assert_eq!(transform_text(&responses).as_deref(), Some("hi"));
        let responses = vec![json!("plain string")];
        assert_eq!(transform_text(&responses).as_deref(), Some("plain string"));
        assert!(transform_text(&[]).is_none());
    }

    #[test]
    fn context_injections_collected() {
        let responses = vec![json!({"context": "a"}), json!({}), json!({"context": "b"})];
        assert_eq!(context_injections(&responses), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn hook_payload_shape() {
        let payload = hook_payload(
            "pre_tool_call",
            "sess1",
            Path::new("/tmp"),
            vec![("tool_name", json!("terminal")), ("tool_input", json!({"command": "ls"}))],
            json!({"turn_id": 3}),
        );
        assert_eq!(payload["hook_event_name"], json!("pre_tool_call"));
        assert_eq!(payload["tool_name"], json!("terminal"));
        assert_eq!(payload["session_id"], json!("sess1"));
        assert_eq!(payload["extra"]["turn_id"], json!(3));
    }

    #[test]
    fn valid_hooks_match_hermes() {
        // Spot-check the hermes VALID_HOOKS set is fully enumerated.
        for hook in [
            "pre_tool_call",
            "post_tool_call",
            "transform_llm_output",
            "pre_verify",
            "pre_gateway_dispatch",
            "kanban_task_blocked",
        ] {
            assert!(VALID_HOOKS.contains(&hook), "missing hook {hook}");
        }
        assert_eq!(VALID_HOOKS.len(), 23);
    }

    #[tokio::test]
    async fn run_hook_callback_parses_stdout() {
        if cfg!(windows) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ulnclaw-hook-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("hook.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ncat > /dev/null\necho '{\"decision\": \"block\", \"reason\": \"test-block\"}'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let callback = HookCallback {
            event: "pre_tool_call".into(),
            command: script.display().to_string(),
            needs_consent: false,
        };
        let response = run_hook_callback(&callback, &json!({"hook_event_name": "pre_tool_call"})).await;
        assert_eq!(
            response.and_then(|r| r.get("reason").and_then(|v| v.as_str()).map(String::from)),
            Some("test-block".to_string())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn run_hook_callback_timeout_is_noop() {
        if cfg!(windows) {
            return;
        }
        let callback = HookCallback {
            event: "pre_tool_call".into(),
            command: "/nonexistent-ulnclaw-hook".into(),
            needs_consent: false,
        };
        assert!(run_hook_callback(&callback, &json!({})).await.is_none());
    }
}
