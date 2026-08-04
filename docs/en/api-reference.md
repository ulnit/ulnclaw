# API Reference

Complete API documentation for ulnclaw.

## Core Types

### Agent

Main orchestrator for AI conversations.

```rust
pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Arc<Mutex<ToolRegistry>>,
    config: AgentConfig,
    callbacks: Arc<Mutex<AgentCallbacks>>,
}
```

**Methods:**

#### `new(provider: Arc<dyn Provider>, tools: ToolRegistry) -> Self`

Create a new agent with a provider and tool registry.

**Parameters:**
- `provider` - AI model provider (wrapped in Arc for shared ownership)
- `tools` - Tool registry containing available tools

**Example:**
```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key("sk-...")
    .model("gpt-4")
    .build()?;

let tools = ToolRegistry::new();
let agent = Agent::new(Arc::new(provider), tools);
```

#### `with_config(self, config: AgentConfig) -> Self`

Set agent configuration.

**Parameters:**
- `config` - Agent configuration struct

**Example:**
```rust
let agent = agent.with_config(AgentConfig {
    max_iterations: 100,
    max_tokens: Some(4096),
    temperature: Some(0.7),
    system_prompt: Some("You are helpful.".into()),
    ..Default::default()
});
```

#### `with_callbacks(self, callbacks: AgentCallbacks) -> Self`

Set event callbacks for UI integration.

**Parameters:**
- `callbacks` - Callback functions for various events

#### `async run(user_message: &str, conversation_history: Option<Vec<Message>>) -> Result<RunResult>`

Run the agent with a user message and optional conversation history.

**Parameters:**
- `user_message` - User's input message
- `conversation_history` - Optional previous messages

**Returns:**
- `RunResult` containing final response, history, usage stats

**Example:**
```rust
let result = agent.run("What's the weather?", None).await?;
println!("{}", result.content);
println!("Tokens used: {}", result.usage.total_tokens);
```

#### `async chat(user_message: &str) -> Result<String>`

Simple interface that returns only the text response.

**Parameters:**
- `user_message` - User's input message

**Returns:**
- Final text response as String

**Example:**
```rust
let response = agent.chat("Hello!").await?;
println!("{}", response);
```

### AgentConfig

Configuration for the agent.

```rust
pub struct AgentConfig {
    pub max_iterations: usize,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub system_prompt: Option<String>,
    pub strip_thinking_blocks: bool,
    pub model: Option<String>,
    pub concurrent_tool_execution: bool,
    pub max_concurrent_tools: usize,
}
```

**Fields:**
- `max_iterations` - Maximum tool calling iterations (default: 50)
- `max_tokens` - Maximum tokens in response
- `temperature` - Sampling temperature (0.0-2.0)
- `system_prompt` - System message for the agent
- `strip_thinking_blocks` - Remove `<think>` tags (default: true)
- `model` - Model name override
- `concurrent_tool_execution` - Execute tools in parallel (default: false)
- `max_concurrent_tools` - Max parallel tools (default: 5)

### RunResult

Result from running the agent.

```rust
pub struct RunResult {
    pub content: String,
    pub conversation: Vec<Message>,
    pub usage: Usage,
    pub iterations: usize,
    pub tool_calls: Vec<ToolCallRecord>,
}
```

**Fields:**
- `content` - Final text response
- `conversation` - Complete message history
- `usage` - Token usage statistics
- `iterations` - Number of iterations executed
- `tool_calls` - Record of all tool calls made

### ToolCallRecord

Record of a single tool call.

```rust
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: serde_json::Value,
}
```

## Provider Types

### Provider Trait

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse>;
    fn model(&self) -> &str;
    fn name(&self) -> &str;
    async fn is_available(&self) -> bool;
}
```

### OpenAiProvider

OpenAI-compatible provider implementation.

```rust
pub struct OpenAiProvider { /* fields hidden */ }
```

**Builder Methods:**

#### `builder() -> OpenAiProviderBuilder`

Create a new builder.

#### `OpenAiProviderBuilder::endpoint(self, endpoint: &str) -> Self`

Set API endpoint URL.

#### `OpenAiProviderBuilder::api_key(self, key: &str) -> Self`

Set API key.

#### `OpenAiProviderBuilder::model(self, model: &str) -> Self`

Set model name.

#### `OpenAiProviderBuilder::name(self, name: &str) -> Self`

Set provider name (for display).

#### `OpenAiProviderBuilder::max_tokens(self, max_tokens: u32) -> Self`

Set maximum tokens.

#### `OpenAiProviderBuilder::temperature(self, temp: f32) -> Self`

Set sampling temperature.

#### `OpenAiProviderBuilder::build(self) -> Result<OpenAiProvider>`

Build the provider.

**Example:**
```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key("sk-...")
    .model("gpt-4")
    .max_tokens(4096)
    .temperature(0.7)
    .build()?;
```

### Message

Conversation message.

```rust
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}
```

### Role

Message role.

```rust
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}
```

### ToolCall

Tool call requested by the model.

```rust
pub struct ToolCall {
    pub id: String,
    pub call_type: String,
    pub function: FunctionCall,
}
```

### FunctionCall

Function call details.

```rust
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}
```

### Usage

Token usage statistics.

```rust
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
```

**Methods:**

#### `merge(&mut self, other: &Usage)`

Merge usage from another request.

### ProviderConfig

Dynamic provider configuration.

```rust
pub struct ProviderConfig {
    pub name: String,
    pub kind: ProviderKind,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub fallback_providers: Vec<String>,
}
```

**Methods:**

#### `build(&self) -> Result<Box<dyn Provider>>`

Build a provider from this configuration.

### ProviderKind

Supported provider types.

```rust
pub enum ProviderKind {
    OpenAiCompatible,
    Ollama,
    LlamaCpp,
    Anthropic,
    Local,
}
```

## Tool Types

### ToolRegistry

Central registry for all tools.

```rust
pub struct ToolRegistry { /* fields hidden */ }
```

**Methods:**

#### `new() -> Self`

Create a new empty registry.

#### `register(&mut self, tool: Tool)`

Register a tool.

#### `unregister(&mut self, name: &str) -> Option<Tool>`

Unregister a tool by name.

#### `get(&self, name: &str) -> Option<&Tool>`

Get a tool by name.

#### `async dispatch(&self, name: &str, arguments: Value) -> Result<Value>`

Execute a tool with given arguments.

#### `definitions(&self) -> Vec<ToolDefinition>`

Get all enabled tool definitions.

#### `names(&self) -> Vec<String>`

Get all tool names.

#### `has(&self, name: &str) -> bool`

Check if a tool exists.

#### `len(&self) -> usize`

Get number of registered tools.

#### `is_empty(&self) -> bool`

Check if registry is empty.

#### `enable_toolset(&mut self, name: &str)`

Enable a toolset.

#### `disable_toolset(&mut self, name: &str)`

Disable a toolset.

#### `toolset_names(&self) -> Vec<String>`

Get all toolset names.

#### `toolset_tools(&self, toolset: &str) -> Vec<&Tool>`

Get tools in a specific toolset.

### Tool

A registered tool with definition and handler.

```rust
pub struct Tool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub toolset: String,
    pub dangerous: bool,
}
```

### ToolDefinition

Tool schema exposed to the model.

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

### ToolBuilder

Fluent API for creating tools.

#### `tool(name: impl Into<String>) -> ToolBuilder`

Create a new tool builder.

**Builder Methods:**

- `description(self, desc: impl Into<String>) -> Self` - Set description
- `parameters(self, params: Value) -> Self` - Set JSON schema
- `handler<F, Fut>(self, handler: F) -> Self` - Set async handler
- `toolset(self, toolset: impl Into<String>) -> Self` - Set toolset
- `dangerous(self, dangerous: bool) -> Self` - Mark as dangerous
- `build(self) -> Result<Tool>` - Build the tool

**Example:**
```rust
let tool = tool("calculate")
    .description("Perform arithmetic operations")
    .parameters(json!({
        "type": "object",
        "properties": {
            "expression": {"type": "string"}
        },
        "required": ["expression"]
    }))
    .handler(|args| async move {
        let expr = args["expression"].as_str().unwrap();
        // Evaluate expression
        Ok(json!({"result": 42}))
    })
    .toolset("math")
    .build()?;
```

### ToolResult

Result of tool execution.

```rust
pub struct ToolResult {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}
```

**Methods:**

#### `ok(data: Value) -> Self`

Create a successful result.

#### `err(msg: impl Into<String>) -> Self`

Create an error result.

#### `to_value(&self) -> Value`

Convert to JSON value.

## Session Types

### SessionStore Trait

```rust
pub trait SessionStore: Send + Sync {
    fn save_session(&self, session: &Session) -> Result<()>;
    fn load_session(&self, session_id: &str) -> Result<Option<Session>>;
    fn list_sessions(&self, limit: usize) -> Result<Vec<Session>>;
    fn delete_session(&self, session_id: &str) -> Result<()>;
    fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<Session>>;
}
```

### MemorySessionStore

In-memory session storage.

```rust
pub struct MemorySessionStore { /* fields hidden */ }
```

**Methods:**

#### `new() -> Self`

Create a new in-memory store.

Implements `SessionStore` trait.

### Session

Conversation session.

```rust
pub struct Session {
    pub id: String,
    pub conversation_id: String,
    pub messages: Vec<Message>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub parent_id: Option<String>,
    pub metadata: SessionMetadata,
}
```

### SessionMetadata

Session metadata.

```rust
pub struct SessionMetadata {
    pub user_id: Option<String>,
    pub platform: Option<String>,
    pub model: Option<String>,
    pub total_tokens: Option<u32>,
    pub iteration_count: Option<u32>,
}
```

#### `new_session(conversation_id: &str) -> Session`

Create a new session with unique ID.

## Context Types

### PromptBuilder

Build system prompts with tiered assembly.

```rust
pub struct PromptBuilder { /* fields hidden */ }
```

**Methods:**

#### `new() -> Self`

Create a new builder.

#### `identity(self, identity: impl Into<String>) -> Self`

Set agent identity.

#### `tool_guidance(self, guidance: impl Into<String>) -> Self`

Set tool usage guidance.

#### `add_skill(self, skill: impl Into<String>) -> Self`

Add a skill/instruction.

#### `add_context_file(self, content: impl Into<String>) -> Self`

Add context file content.

#### `add_env_hint(self, key: impl Into<String>, value: impl Into<String>) -> Self`

Add environment hint.

#### `memory(self, memory: impl Into<String>) -> Self`

Set persistent memory.

#### `suffix(self, suffix: impl Into<String>) -> Self`

Set custom suffix.

#### `build(&self) -> String`

Build the complete system prompt.

**Example:**
```rust
let prompt = PromptBuilder::new()
    .identity("You are a helpful assistant.")
    .tool_guidance("Use tools when needed.")
    .add_skill("Always be polite.")
    .add_env_hint("OS", "Linux")
    .memory("User prefers dark mode.")
    .build();
```

### ContextCompressor

Context window optimization.

```rust
pub struct ContextCompressor {
    pub max_context_tokens: usize,
    pub target_ratio: f32,
}
```

**Methods:**

#### `estimate_tokens(messages: &[Message]) -> usize`

Estimate token count (rough: 4 chars per token).

#### `needs_compression(&self, messages: &[Message]) -> bool`

Check if compression is needed.

#### `compress(&self, messages: Vec<Message>) -> Vec<Message>`

Compress messages (placeholder implementation).

## Error Types

### AgentError

```rust
pub enum AgentError {
    Provider(String),
    Tool(String),
    ToolNotFound(String),
    Session(String),
    Context(String),
    Config(String),
    IterationLimit(usize),
    Http(reqwest::Error),
    Json(serde_json::Error),
    Internal(String),
}
```

**Helper Methods:**

- `provider(msg: impl Into<String>) -> Self`
- `tool(msg: impl Into<String>) -> Self`
- `session(msg: impl Into<String>) -> Self`
- `config(msg: impl Into<String>) -> Self`

### Result Type

```rust
pub type Result<T> = std::result::Result<T, AgentError>;
```

## Utility Functions

### strip_thinking_blocks(text: &str) -> String

Remove `<think>...</think>` and `<thinking>...</thinking>` blocks from text.

**Example:**
```rust
let cleaned = strip_thinking_blocks("Hello <think>thinking</think> world");
assert_eq!(cleaned, "Hello  world");
```

## Prelude Module

Convenient imports for common use cases.

```rust
pub mod prelude {
    pub use crate::agent::{Agent, AgentConfig, RunResult};
    pub use crate::error::{AgentError, Result};
    pub use crate::provider::openai::OpenAiProvider;
    pub use crate::provider::{Message, Provider, Role};
    pub use crate::tools::{tool, ToolRegistry};
    pub use serde_json::json;
    pub use std::sync::Arc;
}
```

**Usage:**
```rust
use ulnclaw::prelude::*;
```

## Constants

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");
```

## Functions

### version() -> &'static str

Get version string.

## Complete Example

```rust
use ulnclaw::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create provider
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key("sk-...")
        .model("gpt-4")
        .build()?;

    // Create tools
    let mut tools = ToolRegistry::new();
    
    tools.register(tool("get_time")
        .description("Get current time")
        .handler(|_| async {
            Ok(json!({"time": "2026-08-04 12:00:00"}))
        })
        .build()?);

    // Create agent
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("You are helpful.".into()),
            max_iterations: 50,
            ..Default::default()
        });

    // Run
    let result = agent.run("What time is it?", None).await?;
    println!("{}", result.content);
    
    Ok(())
}
```
