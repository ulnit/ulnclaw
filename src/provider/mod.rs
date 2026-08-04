//! Provider abstraction layer - supports multiple AI backends
//!
//! Inspired by Hermes Agent's runtime_provider.py which maps (provider, model)
//! to (api_mode, api_key, base_url).

pub mod anthropic;
pub mod auxiliary;
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

/// Incremental tool-call fragment in a streaming response (OpenAI chunk
/// `choices[0].delta.tool_calls[i]`).
#[derive(Debug, Clone, Default)]
pub struct ToolCallDelta {
    /// Position of this tool call within the message.
    pub index: usize,
    pub id: Option<String>,
    /// Function name fragment (usually arrives in one piece).
    pub name_delta: Option<String>,
    /// JSON arguments fragment (arrives incrementally).
    pub arguments_delta: Option<String>,
}

/// One chunk of a streaming chat completion.
#[derive(Debug, Clone, Default)]
pub struct StreamChunk {
    /// Content (text) delta.
    pub delta_content: Option<String>,
    /// Reasoning/thinking delta (models with extended thinking).
    pub delta_reasoning: Option<String>,
    /// Tool-call fragments.
    pub tool_call_deltas: Vec<ToolCallDelta>,
    /// Set on the final chunk: "stop", "tool_calls", "length", ...
    pub finish_reason: Option<String>,
    /// Usage (final chunk when `stream_options.include_usage`).
    pub usage: Option<Usage>,
    /// Model echoed by the server.
    pub model: Option<String>,
}

/// Accumulate streamed tool-call deltas into complete tool calls.
pub fn assemble_tool_calls(
    deltas: &[ToolCallDelta],
) -> Vec<ToolCall> {
    let mut by_index: std::collections::BTreeMap<
        usize,
        (String, String, String),
    > = std::collections::BTreeMap::new();
    for delta in deltas {
        let entry = by_index
            .entry(delta.index)
            .or_insert_with(|| (String::new(), String::new(), String::new()));
        if let Some(id) = &delta.id {
            entry.0 = id.clone();
        }
        if let Some(name) = &delta.name_delta {
            entry.1.push_str(name);
        }
        if let Some(args) = &delta.arguments_delta {
            entry.2.push_str(args);
        }
    }
    by_index
        .into_values()
        .map(|(id, name, arguments)| ToolCall {
            id,
            call_type: "function".to_string(),
            function: FunctionCall { name, arguments },
        })
        .collect()
}


/// A boxed provider stream.
pub type ProviderStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>;

/// Provider trait - abstraction for AI model backends
///
/// Implementations must be Send + Sync for use across async tasks.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a chat completion request
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse>;

    /// Streaming chat completion (SSE chunks).  Default: unsupported —
    /// callers fall back to `chat_completion`.
    async fn chat_completion_stream(&self, _request: ProviderRequest) -> Result<ProviderStream> {
        Err(AgentError::provider(
            "this provider does not support streaming (chat_completion_stream)",
        ))
    }

    /// Whether `chat_completion_stream` is implemented.
    fn supports_streaming(&self) -> bool {
        false
    }

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
                let mut builder = anthropic::AnthropicProvider::builder()
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
            ProviderKind::Local => Err(AgentError::config(
                "Local provider not yet implemented",
            )),
        }
    }
}
