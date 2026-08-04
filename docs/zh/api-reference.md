# API 参考

ulnclaw 完整 API 文档。

## 目录

- [核心类型](#核心类型)
- [提供商类型](#提供商类型)
- [工具类型](#工具类型)
- [会话类型](#会话类型)
- [上下文类型](#上下文类型)
- [错误类型](#错误类型)
- [实用函数](#实用函数)
- [Prelude 模块](#prelude-模块)

## 核心类型

### Agent

AI 对话的主编排器。

```rust
pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Arc<Mutex<ToolRegistry>>,
    config: AgentConfig,
    callbacks: Arc<Mutex<AgentCallbacks>>,
}
```

**方法：**

#### `new(provider: Arc<dyn Provider>, tools: ToolRegistry) -> Self`

创建新的代理，带有提供商和工具注册表。

**参数：**
- `provider` - AI 模型提供商（用 Arc 包装以共享所有权）
- `tools` - 包含可用工具的工具注册表

**示例：**
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

设置代理配置。

**参数：**
- `config` - 代理配置结构体

**示例：**
```rust
let agent = agent.with_config(AgentConfig {
    max_iterations: 100,
    max_tokens: Some(4096),
    temperature: Some(0.7),
    system_prompt: Some("你是一个有帮助的助手。".into()),
    ..Default::default()
});
```

#### `with_callbacks(self, callbacks: AgentCallbacks) -> Self`

设置 UI 集成的事件回调。

**参数：**
- `callbacks` - 各种事件的回调函数

#### `async run(user_message: &str, conversation_history: Option<Vec<Message>>) -> Result<RunResult>`

使用用户消息和可选对话历史运行代理。

**参数：**
- `user_message` - 用户的输入消息
- `conversation_history` - 可选的先前消息

**返回：**
- `RunResult` 包含最终响应、历史、使用统计

**示例：**
```rust
let result = agent.run("天气怎么样？", None).await?;
println!("{}", result.content);
println!("使用令牌数：{}", result.usage.total_tokens);
```

#### `async chat(user_message: &str) -> Result<String>`

简单接口，只返回文本响应。

**参数：**
- `user_message` - 用户的输入消息

**返回：**
- 最终文本响应作为 String

**示例：**
```rust
let response = agent.chat("你好！").await?;
println!("{}", response);
```

### AgentConfig

代理配置。

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

**字段：**
- `max_iterations` - 最大工具调用迭代次数（默认：50）
- `max_tokens` - 响应中的最大令牌数
- `temperature` - 采样温度（0.0-2.0）
- `system_prompt` - 代理的系统消息
- `strip_thinking_blocks` - 移除 `<think>` 标签（默认：true）
- `model` - 模型名称覆盖
- `concurrent_tool_execution` - 并行执行工具（默认：false）
- `max_concurrent_tools` - 最大并行工具数（默认：5）

### RunResult

运行代理的结果。

```rust
pub struct RunResult {
    pub content: String,
    pub conversation: Vec<Message>,
    pub usage: Usage,
    pub iterations: usize,
    pub tool_calls: Vec<ToolCallRecord>,
}
```

**字段：**
- `content` - 最终文本响应
- `conversation` - 完整消息历史
- `usage` - 令牌使用统计
- `iterations` - 执行的迭代次数
- `tool_calls` - 所有工具调用的记录

### ToolCallRecord

单个工具调用的记录。

```rust
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: serde_json::Value,
}
```

## 提供商类型

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

OpenAI 兼容提供商实现。

```rust
pub struct OpenAiProvider { /* 字段隐藏 */ }
```

**构建器方法：**

#### `builder() -> OpenAiProviderBuilder`

创建新的构建器。

#### `OpenAiProviderBuilder::endpoint(self, endpoint: &str) -> Self`

设置 API 端点 URL。

#### `OpenAiProviderBuilder::api_key(self, key: &str) -> Self`

设置 API 密钥。

#### `OpenAiProviderBuilder::model(self, model: &str) -> Self`

设置模型名称。

#### `OpenAiProviderBuilder::name(self, name: &str) -> Self`

设置提供商名称（用于显示）。

#### `OpenAiProviderBuilder::max_tokens(self, max_tokens: u32) -> Self`

设置最大令牌数。

#### `OpenAiProviderBuilder::temperature(self, temp: f32) -> Self`

设置采样温度。

#### `OpenAiProviderBuilder::build(self) -> Result<OpenAiProvider>`

构建提供商。

**示例：**
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

对话消息。

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

消息角色。

```rust
pub enum Role {
    System,    // 系统
    User,      // 用户
    Assistant, // 助手
    Tool,      // 工具
}
```

### ToolCall

模型请求的工具调用。

```rust
pub struct ToolCall {
    pub id: String,
    pub call_type: String,
    pub function: FunctionCall,
}
```

### FunctionCall

函数调用详情。

```rust
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}
```

### Usage

令牌使用统计。

```rust
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
```

**方法：**

#### `merge(&mut self, other: &Usage)`

合并另一个请求的使用统计。

### ProviderConfig

动态提供商配置。

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

**方法：**

#### `build(&self) -> Result<Box<dyn Provider>>`

从此配置构建提供商。

### ProviderKind

支持的提供商类型。

```rust
pub enum ProviderKind {
    OpenAiCompatible,  // OpenAI 兼容
    Ollama,            // Ollama
    LlamaCpp,          // llama.cpp
    Anthropic,         // Anthropic
    Local,             // 本地
}
```

## 工具类型

### ToolRegistry

所有工具的中心注册表。

```rust
pub struct ToolRegistry { /* 字段隐藏 */ }
```

**方法：**

#### `new() -> Self`

创建新的空注册表。

#### `register(&mut self, tool: Tool)`

注册工具。

#### `unregister(&mut self, name: &str) -> Option<Tool>`

按名称注销工具。

#### `get(&self, name: &str) -> Option<&Tool>`

按名称获取工具。

#### `async dispatch(&self, name: &str, arguments: Value) -> Result<Value>`

使用给定参数执行工具。

#### `definitions(&self) -> Vec<ToolDefinition>`

获取所有启用的工具定义。

#### `names(&self) -> Vec<String>`

获取所有工具名称。

#### `has(&self, name: &str) -> bool`

检查工具是否存在。

#### `len(&self) -> usize`

获取注册的工具数量。

#### `is_empty(&self) -> bool`

检查注册表是否为空。

#### `enable_toolset(&mut self, name: &str)`

启用工具集。

#### `disable_toolset(&mut self, name: &str)`

禁用工具集。

#### `toolset_names(&self) -> Vec<String>`

获取所有工具集名称。

#### `toolset_tools(&self, toolset: &str) -> Vec<&Tool>`

获取特定工具集中的工具。

### Tool

带有定义和处理器的已注册工具。

```rust
pub struct Tool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub toolset: String,
    pub dangerous: bool,
}
```

### ToolDefinition

暴露给模型的工具 Schema。

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

### ToolBuilder

创建工具的流畅 API。

#### `tool(name: impl Into<String>) -> ToolBuilder`

创建新的工具构建器。

**构建器方法：**

- `description(self, desc: impl Into<String>) -> Self` - 设置描述
- `parameters(self, params: Value) -> Self` - 设置 JSON Schema
- `handler<F, Fut>(self, handler: F) -> Self` - 设置异步处理器
- `toolset(self, toolset: impl Into<String>) -> Self` - 设置工具集
- `dangerous(self, dangerous: bool) -> Self` - 标记为危险
- `build(self) -> Result<Tool>` - 构建工具

**示例：**
```rust
let tool = tool("calculate")
    .description("执行算术运算")
    .parameters(json!({
        "type": "object",
        "properties": {
            "expression": {"type": "string"}
        },
        "required": ["expression"]
    }))
    .handler(|args| async move {
        let expr = args["expression"].as_str().unwrap();
        // 计算表达式
        Ok(json!({"result": 42}))
    })
    .toolset("math")
    .build()?;
```

### ToolResult

工具执行结果。

```rust
pub struct ToolResult {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}
```

**方法：**

#### `ok(data: Value) -> Self`

创建成功结果。

#### `err(msg: impl Into<String>) -> Self`

创建错误结果。

#### `to_value(&self) -> Value`

转换为 JSON 值。

## 会话类型

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

内存会话存储。

```rust
pub struct MemorySessionStore { /* 字段隐藏 */ }
```

**方法：**

#### `new() -> Self`

创建新的内存存储。

实现 `SessionStore` trait。

### Session

对话会话。

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

会话元数据。

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

创建具有唯一 ID 的新会话。

## 上下文类型

### PromptBuilder

使用分层组装构建系统提示。

```rust
pub struct PromptBuilder { /* 字段隐藏 */ }
```

**方法：**

#### `new() -> Self`

创建新的构建器。

#### `identity(self, identity: impl Into<String>) -> Self`

设置代理身份。

#### `tool_guidance(self, guidance: impl Into<String>) -> Self`

设置工具使用指导。

#### `add_skill(self, skill: impl Into<String>) -> Self`

添加技能/指令。

#### `add_context_file(self, content: impl Into<String>) -> Self`

添加上下文文件内容。

#### `add_env_hint(self, key: impl Into<String>, value: impl Into<String>) -> Self`

添加环境提示。

#### `memory(self, memory: impl Into<String>) -> Self`

设置持久记忆。

#### `suffix(self, suffix: impl Into<String>) -> Self`

设置自定义后缀。

#### `build(&self) -> String`

构建完整的系统提示。

**示例：**
```rust
let prompt = PromptBuilder::new()
    .identity("你是一个有帮助的助手。")
    .tool_guidance("在需要时使用工具。")
    .add_skill("始终保持礼貌。")
    .add_env_hint("OS", "Linux")
    .memory("用户偏好深色模式。")
    .build();
```

### ContextCompressor

上下文窗口优化。

```rust
pub struct ContextCompressor {
    pub max_context_tokens: usize,
    pub target_ratio: f32,
}
```

**方法：**

#### `estimate_tokens(messages: &[Message]) -> usize`

估计令牌数量（粗略：每令牌 4 个字符）。

#### `needs_compression(&self, messages: &[Message]) -> bool`

检查是否需要压缩。

#### `compress(&self, messages: Vec<Message>) -> Vec<Message>`

压缩消息（占位符实现）。

## 错误类型

### AgentError

```rust
pub enum AgentError {
    Provider(String),       // 提供商错误
    Tool(String),           // 工具错误
    ToolNotFound(String),   // 工具未找到
    Session(String),        // 会话错误
    Context(String),        // 上下文错误
    Config(String),         // 配置错误
    IterationLimit(usize),  // 迭代限制
    Http(reqwest::Error),   // HTTP 错误
    Json(serde_json::Error),// JSON 错误
    Internal(String),       // 内部错误
}
```

**辅助方法：**

- `provider(msg: impl Into<String>) -> Self`
- `tool(msg: impl Into<String>) -> Self`
- `session(msg: impl Into<String>) -> Self`
- `config(msg: impl Into<String>) -> Self`

### Result 类型

```rust
pub type Result<T> = std::result::Result<T, AgentError>;
```

## 实用函数

### strip_thinking_blocks(text: &str) -> String

从文本中移除 `<think>...</think>` 和 `<thinking>...</thinking>` 块。

**示例：**
```rust
let cleaned = strip_thinking_blocks("你好 <think>思考中</think> 世界");
assert_eq!(cleaned, "你好  世界");
```

## Prelude 模块

常用用例的便捷导入。

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

**用法：**
```rust
use ulnclaw::prelude::*;
```

## 常量

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");
```

## 函数

### version() -> &'static str

获取版本字符串。

## 完整示例

```rust
use ulnclaw::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // 创建提供商
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key("sk-...")
        .model("gpt-4")
        .build()?;

    // 创建工具
    let mut tools = ToolRegistry::new();
    
    tools.register(tool("get_time")
        .description("获取当前时间")
        .handler(|_| async {
            Ok(json!({"time": "2026-08-04 12:00:00"}))
        })
        .build()?);

    // 创建代理
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("你是一个有帮助的助手。".into()),
            max_iterations: 50,
            ..Default::default()
        });

    // 运行
    let result = agent.run("现在几点了？", None).await?;
    println!("{}", result.content);
    
    Ok(())
}
```

## 下一步

- 阅读 [架构指南](architecture.md) 了解系统设计
- 阅读 [集成指南](integration.md) 了解在应用中使用 ulnclaw
- 阅读 [工具系统](tools.md) 了解构建自定义工具
- 阅读 [提供商系统](providers.md) 了解实现新提供商
