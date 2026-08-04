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
    #[serde(default)]
    pub agent: AgentSettings,
    #[serde(default)]
    pub delegation: DelegationConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
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
        match self.model.provider.as_str() {
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
