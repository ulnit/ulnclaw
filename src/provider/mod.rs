//! Provider abstraction layer - supports multiple AI backends
//!
//! Inspired by Hermes Agent's runtime_provider.py which maps (provider, model)
//! to (api_mode, api_key, base_url).

pub mod openai;

use crate::error::{AgentError, Result};
use crate::tools::ToolDefinition;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Name of the tool (for tool role messages)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Role in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::System => write!(f, "system"),
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::Tool => write!(f, "tool"),
        }
    }
}

/// A tool call requested by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_function_type")]
    pub call_type: String,
    pub function: FunctionCall,
}

fn default_function_type() -> String {
    "function".to_string()
}

/// Function call details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Request to a provider
#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub stop: Option<Vec<String>>,
}

/// Response from a provider
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub model: String,
    /// Reasoning/thinking content (for models that support extended thinking)
    pub reasoning: Option<String>,
    /// Finish reason: "stop", "tool_calls", "length", etc.
    pub finish_reason: Option<String>,
}

/// Token usage statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl Usage {
    pub fn merge(&mut self, other: &Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
    }
}

/// Provider trait - abstraction for AI model backends
///
/// Implementations must be Send + Sync for use across async tasks.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a chat completion request
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse>;

    /// Get the model name
    fn model(&self) -> &str;

    /// Get the provider name
    fn name(&self) -> &str;

    /// Check if provider is available
    async fn is_available(&self) -> bool {
        true
    }

    /// Analyze an image (vision). `image_url` is either an http(s) URL or a
    /// data: URL. Default: unsupported.
    async fn analyze_image(&self, _prompt: &str, _image_url: &str) -> Result<String> {
        Err(AgentError::provider(
            "this provider does not support vision (analyze_image)",
        ))
    }
}

/// Provider configuration for dynamic instantiation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub kind: ProviderKind,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Fallback providers (by name)
    #[serde(default)]
    pub fallback_providers: Vec<String>,
}

/// Supported provider kinds
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// OpenAI-compatible endpoints (OpenAI, OpenRouter, DashScope, etc.)
    OpenAiCompatible,
    /// Ollama local models
    Ollama,
    /// llama.cpp server
    LlamaCpp,
    /// Anthropic Claude
    Anthropic,
    /// Local model (embedded)
    Local,
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderKind::OpenAiCompatible => write!(f, "openai_compatible"),
            ProviderKind::Ollama => write!(f, "ollama"),
            ProviderKind::LlamaCpp => write!(f, "llama_cpp"),
            ProviderKind::Anthropic => write!(f, "anthropic"),
            ProviderKind::Local => write!(f, "local"),
        }
    }
}

impl ProviderConfig {
    /// Build a Provider from this config
    pub fn build(&self) -> Result<Box<dyn Provider>> {
        match self.kind {
            ProviderKind::OpenAiCompatible | ProviderKind::Ollama | ProviderKind::LlamaCpp => {
                let mut builder = openai::OpenAiProvider::builder()
                    .endpoint(&self.endpoint)
                    .model(&self.model)
                    .name(&self.name);

                if let Some(ref key) = self.api_key {
                    builder = builder.api_key(key);
                }
                if let Some(max_tokens) = self.max_tokens {
                    builder = builder.max_tokens(max_tokens);
                }
                if let Some(temp) = self.temperature {
                    builder = builder.temperature(temp);
                }

                Ok(Box::new(builder.build()?))
            }
            ProviderKind::Anthropic => {
                // Anthropic uses OpenAI-compatible format for now
                // TODO: Implement native Anthropic Messages API
                let mut builder = openai::OpenAiProvider::builder()
                    .endpoint(&self.endpoint)
                    .model(&self.model)
                    .name(&self.name);

                if let Some(ref key) = self.api_key {
                    builder = builder.api_key(key);
                }

                Ok(Box::new(builder.build()?))
            }
            ProviderKind::Local => Err(AgentError::config(
                "Local provider not yet implemented",
            )),
        }
    }
}
