//! Configuration layer — port of hermes_cli/config.py
//!
//! Config lives at `~/.ulnclaw/config.toml`. Environment variables can be
//! placed in `~/.ulnclaw/.env` (KEY=VALUE lines). Values resolve with the
//! precedence: explicit override > environment > .env file > config.toml
//! default.

use crate::error::{AgentError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default model used when nothing is configured (mirrors hermes' default).
pub const DEFAULT_MODEL: &str = "gpt-5.2";
/// Default provider when nothing is configured.
pub const DEFAULT_PROVIDER: &str = "openai";

/// Resolve the ulnclaw home directory (`ULNCLAW_HOME` override or `~/.ulnclaw`).
pub fn ulnclaw_home() -> PathBuf {
    if let Ok(dir) = std::env::var("ULNCLAW_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    // Hermes compatibility: fall back to HERMES_HOME when set so existing
    // installs can share state during migration.
    if let Ok(dir) = std::env::var("HERMES_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ulnclaw")
}

/// Ensure home + standard subdirectories exist.
pub fn ensure_home() -> Result<PathBuf> {
    let home = ulnclaw_home();
    for sub in [
        "",
        "memory",
        "skills",
        "cron",
        "sessions",
        "logs",
        "sandboxes",
    ] {
        let dir = if sub.is_empty() {
            home.clone()
        } else {
            home.join(sub)
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| AgentError::config(format!("cannot create {}: {}", dir.display(), e)))?;
    }
    Ok(home)
}

/// Load a KEY=VALUE env file (`.env`), ignoring comments/blank lines.
pub fn load_env_file(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((key, value)) = line.split_once('=') {
                let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
                map.insert(key.trim().to_string(), value.to_string());
            }
        }
    }
    map
}

/// Config-aware env lookup: process env first, then `~/.ulnclaw/.env`.
/// Port of hermes' `get_env_value`.
pub fn get_env_value(name: &str) -> Option<String> {
    if let Ok(val) = std::env::var(name) {
        if !val.is_empty() {
            return Some(val);
        }
    }
    let env_file = ulnclaw_home().join(".env");
    load_env_file(&env_file).remove(name)
}

/// Web search backend selection (port of `web.backend` config).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebConfig {
    /// Search backend: "duckduckgo" (built-in), "tavily", "brave", "searxng", "auto".
    #[serde(default)]
    pub search_backend: Option<String>,
    /// Extract backend: "http" (built-in fetch+strip), "firecrawl", ...
    #[serde(default)]
    pub extract_backend: Option<String>,
}

/// Model/provider settings (port of the model section of hermes config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Fallback chain of "provider:model" strings.
    #[serde(default)]
    pub fallbacks: Vec<String>,
    /// Retry transient provider errors (429/5xx/network) this many times
    /// with exponential backoff before giving up.
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
}

fn default_max_retries() -> usize {
    2
}

fn default_provider() -> String {
    DEFAULT_PROVIDER.to_string()
}
fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            base_url: None,
            api_key: None,
            temperature: None,
            max_tokens: None,
            fallbacks: Vec::new(),
            max_retries: default_max_retries(),
        }
    }
}

/// Agent behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Max tool-call iterations per run (hermes: iteration budget).
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Whether to ask for confirmation before dangerous commands.
    #[serde(default = "default_true")]
    pub approval: bool,
    /// Execute independent tool calls concurrently.
    #[serde(default)]
    pub concurrent_tool_execution: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tools: usize,
    /// Context window budget (tokens) before compression kicks in.
    #[serde(default = "default_context_budget")]
    pub context_budget_tokens: usize,
    /// Verbose logging to stderr.
    #[serde(default)]
    pub verbose: bool,
    /// Probe the local Python toolchain for the system prompt
    /// (hermes `agent.environment_probe`, default true).
    #[serde(default = "default_true")]
    pub environment_probe: bool,
}

fn default_max_iterations() -> usize {
    90
}
fn default_max_concurrent() -> usize {
    5
}
fn default_context_budget() -> usize {
    120_000
}
fn default_true() -> bool {
    true
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            approval: true,
            concurrent_tool_execution: false,
            max_concurrent_tools: default_max_concurrent(),
            context_budget_tokens: default_context_budget(),
            verbose: false,
            environment_probe: true,
        }
    }
}

/// Delegation limits (port of hermes `delegation` config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationConfig {
    #[serde(default = "default_max_children")]
    pub max_concurrent_children: usize,
    #[serde(default = "default_child_iterations")]
    pub child_max_iterations: usize,
    /// Max depth for nested delegation.
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
}

fn default_max_children() -> usize {
    3
}
fn default_child_iterations() -> usize {
    30
}
fn default_max_depth() -> usize {
    1
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            max_concurrent_children: default_max_children(),
            child_max_iterations: default_child_iterations(),
            max_depth: default_max_depth(),
        }
    }
}

/// Terminal tool settings (port of TERMINAL_* env defaults).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    #[serde(default = "default_terminal_timeout")]
    pub timeout: u64,
    /// Foreground max timeout in seconds (hermes FOREGROUND_MAX_TIMEOUT).
    #[serde(default = "default_fg_max")]
    pub foreground_max_timeout: u64,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Execution backend: "local" (default), "docker", or "ssh".
    #[serde(default)]
    pub backend: Option<String>,
    /// Docker container name (backend="docker").
    #[serde(default)]
    pub container: Option<String>,
    /// Docker image to auto-create the container from when missing.
    #[serde(default)]
    pub image: Option<String>,
    /// SSH host (backend="ssh").
    #[serde(default)]
    pub ssh_host: Option<String>,
    /// SSH user (backend="ssh").
    #[serde(default)]
    pub ssh_user: Option<String>,
    /// SSH port (backend="ssh").
    #[serde(default)]
    pub ssh_port: Option<u16>,
    /// Env vars explicitly allowed through the sandbox credential scrub
    /// (hermes `terminal.env_passthrough`).
    #[serde(default)]
    pub env_passthrough: Vec<String>,
    /// SSH identity file (backend="ssh").
    #[serde(default)]
    pub ssh_identity: Option<String>,
}

fn default_terminal_timeout() -> u64 {
    180
}
fn default_fg_max() -> u64 {
    600
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            timeout: default_terminal_timeout(),
            foreground_max_timeout: default_fg_max(),
            cwd: None,
            backend: None,
            container: None,
            image: None,
            ssh_host: None,
            ssh_user: None,
            ssh_port: None,
            ssh_identity: None,
            env_passthrough: Vec::new(),
        }
    }
}

/// Memory limits (port of MemoryStore defaults: 2200/1375 chars).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_limit")]
    pub memory_char_limit: usize,
    #[serde(default = "default_user_limit")]
    pub user_char_limit: usize,
}

fn default_memory_limit() -> usize {
    2200
}
fn default_user_limit() -> usize {
    1375
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            memory_char_limit: default_memory_limit(),
            user_char_limit: default_user_limit(),
        }
    }
}

/// Lenient boolean toggle — accepts a real TOML boolean or one of the
/// common string spellings (hermes `is_truthy_value`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Truthiness {
    Flag(bool),
    Text(String),
}

impl Truthiness {
    /// Resolve to a concrete bool; unrecognized text falls back to
    /// `default` (hermes `is_truthy_value(value, default=...)`).
    pub fn resolve(&self, default: bool) -> bool {
        match self {
            Truthiness::Flag(flag) => *flag,
            Truthiness::Text(text) => {
                match text.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" | "y" | "t" => true,
                    "0" | "false" | "no" | "off" | "n" | "f" | "" => false,
                    _ => default,
                }
            }
        }
    }
}

/// Per-task auxiliary provider override (port of hermes' `auxiliary.<task>`
/// config consumed by `agent/auxiliary_client.py`).
///
/// Tasks are auxiliary LLM calls such as `compression` (context
/// summarization), `vision` (image analysis), and `title_generation`
/// (session titles). When an entry is present,
/// that task is routed through the configured provider/model instead of the
/// main runtime. Blank values and `"auto"` inherit the main runtime.
///
/// `title_generation` additionally reads `enabled` (kill switch, default
/// true) and `language` (pin the title language instead of matching the
/// user's) — hermes `auxiliary.title_generation.{enabled,language}`.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuxiliaryTaskConfig {
    /// Provider name ("openai", "anthropic", "ollama", ...); "auto"/blank
    /// inherits the main provider.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model id; "auto"/blank inherits the main model.
    #[serde(default)]
    pub model: Option<String>,
    /// Endpoint override; blank uses the provider default.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key; blank falls back to `key_env`, then the main runtime key.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable holding the API key.
    #[serde(default)]
    pub key_env: Option<String>,
    /// Task kill switch — only `title_generation` reads this (hermes
    /// `auxiliary.title_generation.enabled`, default true).
    #[serde(default)]
    pub enabled: Option<Truthiness>,
    /// Title language pin — only `title_generation` reads this (hermes
    /// `auxiliary.title_generation.language`); blank matches the user's
    /// language.
    #[serde(default)]
    pub language: Option<String>,
    /// Task output budget override (hermes `auxiliary.goal_judge.max_tokens`);
    /// unset or non-positive falls back to the task's default.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl AuxiliaryTaskConfig {
    fn nonblank(value: Option<&String>) -> Option<String> {
        value
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// Provider override, with "auto" normalized to "inherit".
    pub fn provider(&self) -> Option<String> {
        Self::nonblank(self.provider.as_ref()).filter(|v| v != "auto")
    }

    /// Model override, with "auto" normalized to "inherit".
    pub fn model(&self) -> Option<String> {
        Self::nonblank(self.model.as_ref()).filter(|v| v != "auto")
    }

    pub fn base_url(&self) -> Option<String> {
        Self::nonblank(self.base_url.as_ref())
    }

    pub fn api_key(&self) -> Option<String> {
        Self::nonblank(self.api_key.as_ref())
    }

    /// Resolve the task API key: literal `api_key`, else `key_env` lookup.
    pub fn resolved_api_key(&self) -> Option<String> {
        if let Some(key) = self.api_key() {
            return Some(key);
        }
        Self::nonblank(self.key_env.as_ref()).and_then(|name| get_env_value(&name))
    }

    /// `title_generation` kill switch (hermes
    /// `auxiliary.title_generation.enabled`); defaults to true.
    pub fn enabled(&self) -> bool {
        self.enabled.as_ref().map(|v| v.resolve(true)).unwrap_or(true)
    }

    /// `title_generation` language pin (hermes
    /// `auxiliary.title_generation.language`); blank = match the user.
    pub fn language(&self) -> Option<String> {
        Self::nonblank(self.language.as_ref())
    }

    /// Task output budget override (hermes `_goal_judge_max_tokens`);
    /// non-positive values fall back to the task default.
    pub fn max_tokens(&self) -> Option<u32> {
        self.max_tokens.filter(|v| *v > 0)
    }

    /// True when the entry carries no usable override at all.
    pub fn is_empty(&self) -> bool {
        self.provider().is_none()
            && self.model().is_none()
            && self.base_url().is_none()
            && self.api_key().is_none()
            && Self::nonblank(self.key_env.as_ref()).is_none()
    }
}

/// Mixture-of-Agents slot — one provider/model selection (reference or
/// aggregator). Port of hermes' MoA preset slots (`moa_config.py`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoaSlot {
    pub provider: String,
    pub model: String,
    /// Disabled slots are skipped in the fan-out (hermes `enabled`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Endpoint override; blank uses the provider default.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key; blank falls back to `key_env`, then the main runtime key.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable holding the API key.
    #[serde(default)]
    pub key_env: Option<String>,
}

impl MoaSlot {
    /// `provider:model` display label (hermes `_slot_label`).
    pub fn label(&self) -> String {
        format!("{}:{}", self.provider.trim(), self.model.trim())
    }

    /// Resolve the slot API key: literal, else `key_env`, else main key.
    pub fn resolved_api_key(&self, config: &UlncLawConfig) -> Option<String> {
        if let Some(key) = self.api_key.as_ref().map(|k| k.trim()).filter(|k| !k.is_empty()) {
            return Some(key.to_string());
        }
        if let Some(name) = self.key_env.as_ref().map(|k| k.trim()).filter(|k| !k.is_empty()) {
            if let Some(key) = get_env_value(name) {
                return Some(key);
            }
        }
        config.resolve_api_key()
    }
}

/// One MoA preset: fan-out references + a synthesizing aggregator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoaPreset {
    /// Reference models run in parallel on the user prompt.
    #[serde(default)]
    pub reference_models: Vec<MoaSlot>,
    /// Aggregator synthesizes the reference outputs.
    pub aggregator: MoaSlot,
    /// Optional sampling overrides for the reference fan-out.
    #[serde(default)]
    pub reference_temperature: Option<f32>,
    #[serde(default)]
    pub reference_max_tokens: Option<u32>,
    #[serde(default)]
    pub aggregator_temperature: Option<f32>,
    /// "loud" (default) reports failed references; "silent" hides them.
    #[serde(default = "default_degraded_policy")]
    pub degraded_reference_policy: String,
}

fn default_degraded_policy() -> String {
    "loud".to_string()
}

/// `[moa]` config section (hermes `moa:` presets).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MoaConfig {
    /// Preset used when none is requested (hermes `default_preset`).
    #[serde(default)]
    pub default_preset: Option<String>,
    /// Named presets.
    #[serde(default)]
    pub presets: HashMap<String, MoaPreset>,
}

impl MoaConfig {
    /// Resolve a preset by name, falling back to `default_preset` then
    /// `"default"` (hermes `resolve_moa_preset` semantics).
    pub fn resolve_preset(&self, name: Option<&str>) -> Result<(String, &MoaPreset)> {
        let wanted = name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.default_preset
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "default".to_string());
        if let Some(preset) = self.presets.get(&wanted) {
            return Ok((wanted, preset));
        }
        let mut available: Vec<&String> = self.presets.keys().collect();
        available.sort();
        let listed = if available.is_empty() {
            "(none configured)".to_string()
        } else {
            available
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        Err(AgentError::config(format!(
            "MoA preset '{}' was not found. Available presets: {}",
            wanted, listed
        )))
    }
}


/// HTTP gateway settings (`[gateway]`) — port of hermes' api_server platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_host")]
    pub host: String,
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    /// Bearer token required on API routes (env ULNCLAW_GATEWAY_KEY wins).
    #[serde(default)]
    pub key: Option<String>,
    /// Serve every route additionally under `/p/<profile>/...` mirrors,
    /// each backed by its own agent/store for that `[profiles.<name>]`
    /// override (hermes `gateway.multiplex_profiles`). Off by default:
    /// the prefix is accepted but ignored (single-profile behavior).
    #[serde(default)]
    pub multiplex_profiles: bool,
    /// Cap on concurrently active chat sessions across all surfaces
    /// (hermes `max_concurrent_sessions`; 0/unset disables the cap).
    #[serde(default)]
    pub max_concurrent_sessions: Option<u32>,
}

fn default_gateway_host() -> String {
    "127.0.0.1".to_string()
}

fn default_gateway_port() -> u16 {
    8642
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: default_gateway_host(),
            port: default_gateway_port(),
            key: None,
            multiplex_profiles: false,
            max_concurrent_sessions: None,
        }
    }
}

impl GatewayConfig {
    /// Resolve effective settings with environment overrides.
    pub fn resolved(&self) -> Self {
        let mut config = self.clone();
        if let Ok(host) = std::env::var("ULNCLAW_GATEWAY_HOST") {
            if !host.trim().is_empty() {
                config.host = host.trim().to_string();
            }
        }
        if let Ok(port) = std::env::var("ULNCLAW_GATEWAY_PORT") {
            if let Ok(port) = port.trim().parse() {
                config.port = port;
            }
        }
        if let Ok(key) = std::env::var("ULNCLAW_GATEWAY_KEY") {
            if !key.trim().is_empty() {
                config.key = Some(key.trim().to_string());
            }
        }
        config
    }
}

/// Root config — `~/.ulnclaw/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UlncLawConfig {
    #[serde(default)]
    pub model: ModelConfig,
    /// IANA timezone for prompt timestamps (hermes config.yaml
    /// `timezone`); `ULNCLAW_TIMEZONE`/`HERMES_TIMEZONE` env vars take
    /// priority, blank/unset falls back to server-local time.
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub agent: AgentSettings,
    #[serde(default)]
    pub delegation: DelegationConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    /// Transparent filesystem checkpoints (hermes checkpoint_manager).
    #[serde(default)]
    pub checkpoints: crate::checkpoint::CheckpointsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub web: WebConfig,
    /// Enabled toolsets (empty = default "coding" set).
    #[serde(default)]
    pub enabled_toolsets: Vec<String>,
    /// Disabled toolsets.
    #[serde(default)]
    pub disabled_toolsets: Vec<String>,
    /// Named profiles (hermes profiles).
    #[serde(default)]
    pub profiles: HashMap<String, ProfileOverride>,
    /// MCP server connections.
    #[serde(default)]
    pub mcp: McpConfig,
    /// HTTP gateway (OpenAI-compatible API server).
    #[serde(default)]
    pub gateway: GatewayConfig,
    /// Approval flow settings (hermes `[approvals]`).
    #[serde(default)]
    pub approvals: ApprovalsConfig,
    /// Per-task auxiliary provider overrides (hermes `auxiliary.<task>`):
    /// `[auxiliary.compression]`, `[auxiliary.vision]`, ...
    #[serde(default)]
    pub auxiliary: HashMap<String, AuxiliaryTaskConfig>,
    /// Mixture-of-Agents presets (`[moa]`, hermes `moa:`).
    #[serde(default)]
    pub moa: MoaConfig,
    /// Tool-output truncation limits (hermes `tool_output:`).
    #[serde(default)]
    pub tool_output: ToolOutputConfig,
    /// URL/SSRF safety settings (hermes `security:`).
    #[serde(default)]
    pub security: SecurityConfig,
    /// Presentation toggles for GUI hosts (hermes `display:`).
    #[serde(default)]
    pub display: DisplayConfig,
    /// X (Twitter) search via xAI (hermes `x_search:`).
    #[serde(default)]
    pub x_search: XSearchConfig,
    /// Video generation backend selection (hermes `video_gen:`).
    #[serde(default)]
    pub video_gen: VideoGenConfig,
    /// External secret sources (hermes `secrets.*`).
    #[serde(default)]
    pub secrets: crate::secrets::SecretsConfig,
    /// Computer-use / cua-driver settings (hermes `computer_use:`).
    #[serde(default)]
    pub computer_use: crate::computer_use::ComputerUseConfig,
    /// Plugin system settings (hermes `plugins` deny-list).
    #[serde(default)]
    pub plugins: crate::plugins::PluginsConfig,
    /// Shell-hook commands per lifecycle event (hermes `hooks:` block).
    #[serde(default)]
    pub hooks: crate::plugins::HooksConfig,
    /// Messaging platform gateways (hermes gateway/platforms).
    #[serde(default)]
    pub messaging: crate::messaging::MessagingConfig,
    /// Speech-to-text pipeline for voice messages (hermes `stt:`).
    #[serde(default)]
    pub stt: crate::stt::SttConfig,
    /// OAuth device-flow login (hermes portal auth, service-agnostic).
    #[serde(default)]
    pub oauth: crate::oauth::OAuthConfig,
    /// Skill sync across devices (hermes `hermes sync`).
    #[serde(default)]
    pub sync: crate::skills_sync::SyncConfig,
    /// Pet hatch image-generation endpoint overrides (hermes
    /// `agent/pet/generate` provider config).
    #[serde(default)]
    pub pets: PetsConfig,
    /// Kanban dispatcher settings (hermes `kanban:` block).
    #[serde(default)]
    pub kanban: KanbanConfig,
}

/// `[kanban]` — dispatcher settings (hermes `kanban.*`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KanbanConfig {
    /// Run dispatcher ticks inside the gateway (hermes
    /// `kanban.dispatch_in_gateway`, default on).
    pub dispatch_in_gateway: bool,
    /// Seconds between dispatch ticks (hermes default 60).
    pub dispatch_interval_secs: u64,
    /// Max concurrent kanban workers (live concurrency cap).
    pub max_spawn: usize,
    /// Spawn each worker in its own git worktree when dispatching from a
    /// repo (hermes worktree workspaces; falls back to in-place otherwise).
    pub worktrees: bool,
}

impl Default for KanbanConfig {
    fn default() -> Self {
        Self {
            dispatch_in_gateway: true,
            dispatch_interval_secs: 60,
            max_spawn: 2,
            worktrees: true,
        }
    }
}

/// `[pets]` — image-generation endpoint overrides for the pet hatch
/// pipeline (hermes `agent/pet/generate/imagegen.py` provider settings).
/// All optional: key falls back to `OPENAI_API_KEY`/`ULNCLAW_API_KEY`,
/// base URL to `https://api.openai.com/v1`, model to `gpt-image-2`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PetsConfig {
    /// OpenAI-compatible base URL serving `/images/generations` and
    /// `/images/edits`.
    pub image_base_url: Option<String>,
    /// API key for the images endpoint.
    pub image_api_key: Option<String>,
    /// Image model id (hermes default `gpt-image-2`).
    pub image_model: Option<String>,
}

/// `[video_gen]` — video generation backend selection (hermes config.yaml
/// `video_gen:`). The provider names a registered `VideoGenProvider`;
/// blank auto-selects when exactly one available provider is registered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoGenConfig {
    /// Active backend name (`xai`, `fal`, `deepinfra`, ...); blank
    /// auto-selects a single available provider.
    #[serde(default)]
    pub provider: Option<String>,
    /// Default model for the active backend; blank uses the provider's
    /// default.
    #[serde(default)]
    pub model: Option<String>,
    /// FAL-specific settings (hermes `video_gen.fal:`).
    #[serde(default)]
    pub fal: FalVideoGenConfig,
}

/// `[video_gen.fal]` — FAL backend tuning (hermes `video_gen.fal:`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FalVideoGenConfig {
    /// FAL model family id (`pixverse-v6`, `veo3.1`, ...); overrides the
    /// top-level `video_gen.model` when it names a known family.
    #[serde(default)]
    pub model: Option<String>,
}

/// `[x_search]` — xAI X-search tuning (port of hermes `x_search.*`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XSearchConfig {
    /// Responses-API model used for the x_search server tool
    /// (hermes default `grok-4.5`).
    pub model: String,
    /// Optional reasoning effort: low | medium | high | xhigh.
    pub reasoning_effort: String,
    /// Request timeout in seconds (floor 30, hermes default 180).
    pub timeout_seconds: u64,
    /// Retries on 5xx / transient failures (hermes default 2).
    pub retries: u32,
}

impl Default for XSearchConfig {
    fn default() -> Self {
        Self {
            model: "grok-4.5".to_string(),
            reasoning_effort: String::new(),
            timeout_seconds: 180,
            retries: 2,
        }
    }
}

/// `[display]` — presentation toggles for GUI hosts (hermes `display.*`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DisplayConfig {
    /// Enable message reactions (tapbacks) in desktop GUIs — off by
    /// default, enabled from the desktop app's settings (hermes
    /// `display.message_reactions`).
    pub message_reactions: bool,
    /// Active theme name — one of the built-in skins (default, ares, mono,
    /// slate, daylight, warm-lightmode, poseidon, sisyphus, charizard);
    /// blank/unset = `default` (hermes `display.skin`).
    #[serde(default)]
    pub skin: Option<String>,
    /// Petdex mascot display settings (hermes `display.pet.*`).
    #[serde(default)]
    pub pet: PetDisplayConfig,
}

/// `[display.pet]` — petdex mascot display (hermes `display.pet.*`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PetDisplayConfig {
    /// Whether the pet display is on (`display.pet.enabled`).
    pub enabled: bool,
    /// Active pet slug under `<home>/pets/` (`display.pet.slug`).
    #[serde(default)]
    pub slug: Option<String>,
    /// Master scale 0.1–3.0 shared by every surface (`display.pet.scale`).
    #[serde(default)]
    pub scale: Option<f64>,
    /// Render mode override: auto/kitty/iterm/sixel/unicode/off
    /// (`display.pet.render_mode`).
    #[serde(default)]
    pub render_mode: Option<String>,
    /// Explicit half-block column width override (`display.pet.unicode_cols`).
    #[serde(default)]
    pub unicode_cols: Option<u32>,
}

impl Default for PetDisplayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            slug: None,
            scale: None,
            render_mode: None,
            unicode_cols: None,
        }
    }
}

/// `[security]` — URL safety toggles (port of hermes `security.*`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SecurityConfig {
    /// Allow web tools to fetch private/internal addresses (disables the
    /// SSRF block). Cloud metadata endpoints remain blocked regardless
    /// (hermes `security.allow_private_urls`).
    pub allow_private_urls: bool,
}

/// `[tool_output]` — configurable tool-output truncation limits (port of
/// hermes `tools/tool_output_limits.py`). Defaults preserve the existing
/// ulnclaw behaviour; invalid or non-positive values fall back to the
/// defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolOutputConfig {
    /// Terminal stdout/stderr character cap (head+tail kept).
    #[serde(default = "default_tool_output_max_bytes")]
    pub max_bytes: usize,
    /// read_file pagination/truncation line cap.
    #[serde(default = "default_tool_output_max_lines")]
    pub max_lines: usize,
    /// Per-line length cap before '... [truncated]'.
    #[serde(default = "default_tool_output_max_line_length")]
    pub max_line_length: usize,
}

fn default_tool_output_max_bytes() -> usize {
    100_000
}
fn default_tool_output_max_lines() -> usize {
    2000
}
fn default_tool_output_max_line_length() -> usize {
    2000
}

fn positive_or(value: usize, default: usize) -> usize {
    if value == 0 { default } else { value }
}

impl ToolOutputConfig {
    /// Resolved limits with non-positive values coerced to defaults
    /// (hermes `_coerce_positive_int` semantics).
    pub fn resolved(&self) -> ToolOutputConfig {
        ToolOutputConfig {
            max_bytes: positive_or(self.max_bytes, default_tool_output_max_bytes()),
            max_lines: positive_or(self.max_lines, default_tool_output_max_lines()),
            max_line_length: positive_or(self.max_line_length, default_tool_output_max_line_length()),
        }
    }
}

impl Default for ToolOutputConfig {
    fn default() -> Self {
        Self {
            max_bytes: default_tool_output_max_bytes(),
            max_lines: default_tool_output_max_lines(),
            max_line_length: default_tool_output_max_line_length(),
        }
    }
}

/// `[approvals]` config section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalsConfig {
    /// Seconds to wait for a human decision before fail-closed auto-deny
    /// (hermes default 300).
    pub timeout: u64,
    /// `manual` (prompt a human), `smart` (auxiliary-LLM guardian first,
    /// escalate to a human when unsure) or `off` (auto-approve everything
    /// except the hardline floor) — hermes `approvals.mode`.
    pub mode: String,
    /// `deny` (default, fail-closed) or `approve` — what happens when a
    /// cron-triggered run hits the approval gate with no human present
    /// (hermes `approvals.cron_mode`).
    pub cron_mode: String,
    /// Operator rules appended to the smart-approval guardian's system
    /// prompt (hermes `approvals.smart_policy`).
    pub smart_policy: String,
}

impl Default for ApprovalsConfig {
    fn default() -> Self {
        Self {
            timeout: 300,
            mode: "manual".to_string(),
            cron_mode: "deny".to_string(),
            smart_policy: String::new(),
        }
    }
}

/// MCP configuration section ([[mcp.servers]]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<crate::mcp::McpServerConfig>,
}

/// A named profile override section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileOverride {
    #[serde(default)]
    pub model: Option<ModelConfig>,
    #[serde(default)]
    pub enabled_toolsets: Option<Vec<String>>,
    #[serde(default)]
    pub disabled_toolsets: Option<Vec<String>>,
}

/// Default base URL for a provider name (environment overrides aware).
/// Shared by the main runtime and auxiliary task resolution.
pub fn default_base_url(provider: &str) -> String {
    match provider {
        "openai" => get_env_value("OPENAI_BASE_URL")
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        "anthropic" => "https://api.anthropic.com".to_string(),
        "ollama" => get_env_value("OLLAMA_BASE_URL")
            .unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
        "dashscope" => "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        _ => get_env_value("OPENAI_BASE_URL")
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
    }
}

impl UlncLawConfig {
    /// Load config from the given path (or default location when `None`).
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path
            .map(PathBuf::from)
            .unwrap_or_else(|| ulnclaw_home().join("config.toml"));
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| AgentError::config(format!("read {}: {}", path.display(), e)))?;
        let config: UlncLawConfig = toml::from_str(&content)
            .map_err(|e| AgentError::config(format!("parse {}: {}", path.display(), e)))?;
        Ok(config)
    }

    /// Apply a named profile on top of this config.
    pub fn with_profile(mut self, name: &str) -> Self {
        if let Some(profile) = self.profiles.get(name).cloned() {
            if let Some(model) = profile.model {
                self.model = model;
            }
            if let Some(ts) = profile.enabled_toolsets {
                self.enabled_toolsets = ts;
            }
            if let Some(ts) = profile.disabled_toolsets {
                self.disabled_toolsets = ts;
            }
        }
        self
    }

    /// Resolve the API key: config > ULNCLAW_API_KEY > OPENAI_API_KEY > .env.
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Some(ref key) = self.model.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }
        get_env_value("ULNCLAW_API_KEY")
            .or_else(|| get_env_value("OPENAI_API_KEY"))
            .or_else(|| get_env_value("ANTHROPIC_API_KEY"))
    }

    /// Resolve the base URL for the configured provider.
    pub fn resolve_base_url(&self) -> String {
        if let Some(ref url) = self.model.base_url {
            if !url.is_empty() {
                return url.clone();
            }
        }
        default_base_url(&self.model.provider)
    }

    /// Write a default config file if none exists.
    pub fn write_default_if_missing() -> Result<PathBuf> {
        let home = ensure_home()?;
        let path = home.join("config.toml");
        if !path.exists() {
            let default = UlncLawConfig::default();
            let content = toml::to_string_pretty(&default)
                .map_err(|e| AgentError::config(format!("serialize config: {}", e)))?;
            let header = "# ulnclaw configuration (ported from hermes-agent config.yaml)\n\
                          # See docs/en/configuration.md for all options.\n\n";
            std::fs::write(&path, format!("{}{}", header, content))
                .map_err(|e| AgentError::config(format!("write {}: {}", path.display(), e)))?;
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_tool_output_limits_coercion() {
        let cfg = ToolOutputConfig { max_bytes: 0, max_lines: 0, max_line_length: 0 };
        let r = cfg.resolved();
        assert_eq!(r.max_bytes, 100_000);
        assert_eq!(r.max_lines, 2000);
        assert_eq!(r.max_line_length, 2000);
        let cfg = ToolOutputConfig { max_bytes: 500, max_lines: 10, max_line_length: 80 };
        let r = cfg.resolved();
        assert_eq!((r.max_bytes, r.max_lines, r.max_line_length), (500, 10, 80));
    }
    use super::*;

    #[test]
    fn test_default_config_roundtrip() {
        let config = UlncLawConfig::default();
        let s = toml::to_string(&config).unwrap();
        let back: UlncLawConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.agent.max_iterations, 90);
        assert_eq!(back.memory.memory_char_limit, 2200);
        assert_eq!(back.delegation.max_concurrent_children, 3);
    }

    #[test]
    fn test_env_file_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "# comment\nFOO=bar\nexport BAZ=\"qux\"\n\nEMPTY=\n",
        )
        .unwrap();
        let map = load_env_file(&path);
        assert_eq!(map.get("FOO").unwrap(), "bar");
        assert_eq!(map.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn test_profile_override() {
        let mut config = UlncLawConfig::default();
        let mut profile = ProfileOverride::default();
        let mut model = ModelConfig::default();
        model.model = "test-model".into();
        profile.model = Some(model);
        config.profiles.insert("work".into(), profile);
        let config = config.with_profile("work");
        assert_eq!(config.model.model, "test-model");
    }
}
