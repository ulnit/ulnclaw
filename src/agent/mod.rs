//! Agent - the core conversation loop
//!
//! Inspired by Hermes Agent's AIAgent class (run_agent.py).
//! Handles: provider selection, prompt construction, tool execution,
//! retries, fallback, callbacks, compression, and persistence.

use crate::error::Result;
use crate::provider::{Message, Provider, ProviderRequest, Role, Usage};
use crate::tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Agent configuration
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Maximum iterations (tool call loops) per conversation
    pub max_iterations: usize,
    /// Maximum tokens for responses
    pub max_tokens: Option<u32>,
    /// Temperature for generation
    pub temperature: Option<f32>,
    /// System prompt
    pub system_prompt: Option<String>,
    /// Whether to strip thinking blocks from output
    pub strip_thinking_blocks: bool,
    /// Model name override (uses provider default if None)
    pub model: Option<String>,
    /// Whether to execute tool calls concurrently
    pub concurrent_tool_execution: bool,
    /// Maximum concurrent tool calls
    pub max_concurrent_tools: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            max_tokens: None,
            temperature: None,
            system_prompt: None,
            strip_thinking_blocks: true,
            model: None,
            concurrent_tool_execution: false,
            max_concurrent_tools: 5,
        }
    }
}

/// Callbacks for agent events
pub struct AgentCallbacks {
    /// Called when a tool call starts
    pub on_tool_start: Option<Box<dyn Fn(&str, &serde_json::Value) + Send + Sync>>,
    /// Called when a tool call completes
    pub on_tool_complete: Option<Box<dyn Fn(&str, &serde_json::Value) + Send + Sync>>,
    /// Called for streaming text deltas
    pub on_stream_delta: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Called when model is thinking
    pub on_thinking: Option<Box<dyn Fn() + Send + Sync>>,
    /// Called after each complete agent turn
    pub on_step: Option<Box<dyn Fn(u32) + Send + Sync>>,
}

impl Default for AgentCallbacks {
    fn default() -> Self {
        Self {
            on_tool_start: None,
            on_tool_complete: None,
            on_stream_delta: None,
            on_thinking: None,
            on_step: None,
        }
    }
}

/// Result of running the agent
#[derive(Debug, Clone)]
pub struct RunResult {
    /// The final text response
    pub content: String,
    /// Complete conversation history
    pub conversation: Vec<Message>,
    /// Total token usage across all iterations
    pub usage: Usage,
    /// Number of iterations executed
    pub iterations: usize,
    /// Tool calls made during this run
    pub tool_calls: Vec<ToolCallRecord>,
}

/// Record of a tool call
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: serde_json::Value,
}

/// The main agent - orchestrates conversation flow
pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Arc<Mutex<ToolRegistry>>,
    config: AgentConfig,
    callbacks: Arc<Mutex<AgentCallbacks>>,
}

impl Agent {
    /// Create a new agent with a provider and tool registry
    pub fn new(provider: Arc<dyn Provider>, tools: ToolRegistry) -> Self {
        Self {
            provider,
            tools: Arc::new(Mutex::new(tools)),
            config: AgentConfig::default(),
            callbacks: Arc::new(Mutex::new(AgentCallbacks::default())),
        }
    }

    /// Set agent configuration
    pub fn with_config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    /// Set callbacks
    pub fn with_callbacks(mut self, callbacks: AgentCallbacks) -> Self {
        self.callbacks = Arc::new(Mutex::new(callbacks));
        self
    }

    /// Run the agent with a user message
    ///
    /// This is the main entry point - implements the conversation loop:
    /// 1. Build messages (system prompt + conversation history + user message)
    /// 2. Call provider
    /// 3. If tool calls: execute them, append results, loop back to step 2
    /// 4. If text response: return it
    pub async fn run(
        &self,
        user_message: &str,
        conversation_history: Option<Vec<Message>>,
    ) -> Result<RunResult> {
        let mut messages = Vec::new();
        let mut total_usage = Usage::default();
        let mut tool_calls_made = Vec::new();

        // Add system prompt if configured
        if let Some(ref prompt) = self.config.system_prompt {
            messages.push(Message {
                role: Role::System,
                content: Some(prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        // Add conversation history
        if let Some(history) = conversation_history {
            messages.extend(history);
        }

        // Add user message
        messages.push(Message {
            role: Role::User,
            content: Some(user_message.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        // Main conversation loop
        for iteration in 0..self.config.max_iterations {
            debug!("Agent iteration {}", iteration);

            // Call provider
            let response = self.call_provider(&messages).await?;

            // Merge usage
            if let Some(ref usage) = response.usage {
                total_usage.merge(usage);
            }

            // Check for tool calls
            if !response.tool_calls.is_empty() {
                // Add assistant message with tool calls
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.content,
                    tool_calls: Some(response.tool_calls.clone()),
                    tool_call_id: None,
                    name: None,
                });

                // Execute tool calls
                let tool_results = self.execute_tool_calls(&response.tool_calls).await?;

                // Add tool results to messages and record
                for (tool_call, result) in response.tool_calls.iter().zip(tool_results.iter()) {
                    messages.push(Message {
                        role: Role::Tool,
                        content: Some(serde_json::to_string(result).unwrap_or_default()),
                        tool_calls: None,
                        tool_call_id: Some(tool_call.id.clone()),
                        name: Some(tool_call.function.name.clone()),
                    });

                    // Parse arguments for recording
                    let args: serde_json::Value =
                        serde_json::from_str(&tool_call.function.arguments)
                            .unwrap_or(serde_json::json!({}));

                    tool_calls_made.push(ToolCallRecord {
                        id: tool_call.id.clone(),
                        name: tool_call.function.name.clone(),
                        arguments: args,
                        result: result.clone(),
                    });
                }

                // Notify step callback
                let callbacks = self.callbacks.lock().await;
                if let Some(ref on_step) = callbacks.on_step {
                    on_step(iteration as u32 + 1);
                }

                continue;
            }

            // No tool calls - we have our final response
            let mut content = response.content.unwrap_or_default();

            if self.config.strip_thinking_blocks {
                content = strip_thinking_blocks(&content);
            }

            return Ok(RunResult {
                content,
                conversation: messages,
                usage: total_usage,
                iterations: iteration + 1,
                tool_calls: tool_calls_made,
            });
        }

        // Max iterations reached
        Ok(RunResult {
            content: "达到最大迭代次数限制".to_string(),
            conversation: messages,
            usage: total_usage,
            iterations: self.config.max_iterations,
            tool_calls: tool_calls_made,
        })
    }

    /// Simple interface - just return the text response
    pub async fn chat(&self, user_message: &str) -> Result<String> {
        let result = self.run(user_message, None).await?;
        Ok(result.content)
    }

    /// Call the provider with current messages
    async fn call_provider(&self, messages: &[Message]) -> Result<crate::provider::ProviderResponse> {
        let tools = self.tools.lock().await;
        let tool_definitions = tools.definitions();

        let request = ProviderRequest {
            messages: messages.to_vec(),
            tools: tool_definitions,
            model: self
                .config
                .model
                .clone()
                .unwrap_or_else(|| self.provider.model().to_string()),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            stream: false,
            stop: None,
        };

        self.provider.chat_completion(request).await
    }

    /// Execute tool calls (sequentially or concurrently)
    async fn execute_tool_calls(
        &self,
        tool_calls: &[crate::provider::ToolCall],
    ) -> Result<Vec<serde_json::Value>> {
        let mut results = Vec::new();

        for tool_call in tool_calls {
            let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
                .unwrap_or(serde_json::json!({}));

            // Notify tool start
            {
                let callbacks = self.callbacks.lock().await;
                if let Some(ref on_tool_start) = callbacks.on_tool_start {
                    on_tool_start(&tool_call.function.name, &args);
                }
            }

            // Execute the tool
            let tools = self.tools.lock().await;
            let result = tools.dispatch(&tool_call.function.name, args.clone()).await;

            let result_value = match result {
                Ok(v) => v,
                Err(e) => {
                    warn!("Tool {} failed: {}", tool_call.function.name, e);
                    serde_json::json!({
                        "error": e.to_string(),
                        "success": false,
                    })
                }
            };

            // Notify tool complete
            {
                let callbacks = self.callbacks.lock().await;
                if let Some(ref on_tool_complete) = callbacks.on_tool_complete {
                    on_tool_complete(&tool_call.function.name, &result_value);
                }
            }

            results.push(result_value);
        }

        Ok(results)
    }
}

/// Strip thinking/reasoning blocks from text
/// Removes <thinking>...</thinking> and <think>...</think> blocks
pub fn strip_thinking_blocks(text: &str) -> String {
    let mut result = text.to_string();

    // Remove <thinking>...</thinking> blocks
    while let Some(start) = result.find("<thinking>") {
        if let Some(end) = result[start..].find("</thinking>") {
            result = format!(
                "{}{}",
                &result[..start],
                &result[start + end + "</thinking>".len()..]
            );
        } else {
            result = result[..start].to_string();
            break;
        }
    }

    // Remove <think>...</think> blocks (Qwen3 style)
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result[start..].find("</think>") {
            result = format!(
                "{}{}",
                &result[..start],
                &result[start + end + "</think>".len()..]
            );
        } else {
            result = result[..start].to_string();
            break;
        }
    }

    result.trim().to_string()
}
