# ulnclaw 🦞

[English](#english) | [中文](#中文)

---

## English

**A high-performance, extensible AI agent engine written in Rust**

ulnclaw is a modern AI agent framework inspired by [Hermes Agent](https://github.com/NousResearch/hermes-agent). It provides a complete agent loop with intelligent tool calling, multi-provider support, session persistence, and advanced context management.

### Key Features

- **🤖 Intelligent Agent Loop**: Automatic tool calling with configurable iteration limits and smart termination
- **🔧 Extensible Tool System**: Self-registering tools with JSON schema, async handlers, and toolset management
- **🌐 Multi-Provider Support**: Works with OpenAI, Anthropic, DashScope, Ollama, llama.cpp, and more
- **💾 Session Persistence**: In-memory and SQLite-based conversation storage with lineage tracking
- **🎯 Context Management**: Intelligent prompt building with tiered assembly and compression
- **📡 Real-time Callbacks**: Progress, thinking, streaming, and step callbacks for UI integration
- **🛡️ Robust Error Handling**: Comprehensive error types with automatic retry and provider fallback

### Quick Start

```rust
use ulnclaw::prelude::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // Create provider
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key("sk-...")
        .model("gpt-4")
        .build()?;

    // Create tool registry
    let mut tools = ToolRegistry::new();
    tools.register(tool("get_weather")
        .description("Get current weather")
        .handler(|args| async move {
            let city = args["city"].as_str().unwrap_or("Beijing");
            Ok(json!({"city": city, "temp": "22°C", "condition": "sunny"}))
        })
        .build()?);

    // Create and run agent
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("You are a helpful weather assistant.".into()),
            ..Default::default()
        });

    let result = agent.run("What's the weather in Shanghai?", None).await?;
    println!("{}", result.content);
    
    Ok(())
}
```

### Documentation

- [Architecture Guide](docs/en/architecture.md) - System design and component overview
- [API Reference](docs/en/api-reference.md) - Complete API documentation
- [Integration Guide](docs/en/integration.md) - How to integrate ulnclaw into your project
- [Development Guide](docs/en/development.md) - Contributing and extending ulnclaw
- [Tool System](docs/en/tools.md) - Building custom tools and toolsets
- [Provider System](docs/en/providers.md) - Implementing new AI providers

### Project Structure

```
ulnclaw/
├── src/
│   ├── lib.rs                 # Library entry point
│   ├── agent/mod.rs           # Core agent loop (367 lines)
│   ├── provider/
│   │   ├── mod.rs            # Provider trait and types (214 lines)
│   │   └── openai.rs         # OpenAI-compatible provider (371 lines)
│   ├── tools/mod.rs          # Tool registry system (287 lines)
│   ├── session/mod.rs        # Session persistence (144 lines)
│   ├── context/mod.rs        # Context management (200 lines)
│   └── error.rs              # Error types (74 lines)
├── tests/
│   └── integration_test.rs   # Integration tests (258 lines)
└── docs/                      # Documentation (bilingual)
```

**Total**: 2,027 lines of Rust code | 10 tests (100% passing)

### Building

```bash
# Development build
cargo check

# Run tests
cargo test

# Release build
cargo build --release

# Static binary (musl)
cargo build --release --target x86_64-unknown-linux-musl
```

### License

MIT OR Apache-2.0

---

## 中文

**高性能、可扩展的 Rust AI Agent 引擎**

ulnclaw 是一个现代化的 AI Agent 框架，灵感来自 [Hermes Agent](https://github.com/NousResearch/hermes-agent)。它提供完整的 Agent 循环，包括智能工具调用、多提供商支持、会话持久化和高级上下文管理。

### 核心特性

- **🤖 智能 Agent 循环**：自动工具调用，可配置迭代限制和智能终止
- **🔧 可扩展工具系统**：自注册工具，支持 JSON Schema、异步处理器和工具集管理
- **🌐 多提供商支持**：支持 OpenAI、Anthropic、DashScope、Ollama、llama.cpp 等
- **💾 会话持久化**：内存和 SQLite 存储，支持会话谱系追踪
- **🎯 上下文管理**：智能提示构建，支持分层组装和压缩
- **📡 实时回调**：进度、思考、流式和步骤回调，便于 UI 集成
- **🛡️ 健壮错误处理**：完整的错误类型，支持自动重试和提供商回退

### 快速开始

```rust
use ulnclaw::prelude::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // 创建提供商
    let provider = OpenAiProvider::builder()
        .endpoint("https://dashscope.aliyuncs.com/compatible-mode")
        .api_key("sk-...")
        .model("qwen-plus")
        .build()?;

    // 创建工具注册表
    let mut tools = ToolRegistry::new();
    tools.register(tool("get_weather")
        .description("获取当前天气")
        .handler(|args| async move {
            let city = args["city"].as_str().unwrap_or("北京");
            Ok(json!({"city": city, "temp": "22°C", "condition": "晴朗"}))
        })
        .build()?);

    // 创建并运行 Agent
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("你是一个有帮助的天气助手。".into()),
            ..Default::default()
        });

    let result = agent.run("上海天气怎么样？", None).await?;
    println!("{}", result.content);
    
    Ok(())
}
```

### 文档

- [架构指南](docs/zh/architecture.md) - 系统设计和组件概览
- [API 参考](docs/zh/api-reference.md) - 完整 API 文档
- [集成指南](docs/zh/integration.md) - 如何将 ulnclaw 集成到你的项目
- [开发指南](docs/zh/development.md) - 贡献和扩展 ulnclaw
- [工具系统](docs/zh/tools.md) - 构建自定义工具和工具集
- [提供商系统](docs/zh/providers.md) - 实现新的 AI 提供商

### 项目结构

```
ulnclaw/
├── src/
│   ├── lib.rs                 # 库入口点
│   ├── agent/mod.rs           # 核心 Agent 循环 (367 行)
│   ├── provider/
│   │   ├── mod.rs            # Provider trait 和类型 (214 行)
│   │   └── openai.rs         # OpenAI 兼容提供商 (371 行)
│   ├── tools/mod.rs          # 工具注册系统 (287 行)
│   ├── session/mod.rs        # 会话持久化 (144 行)
│   ├── context/mod.rs        # 上下文管理 (200 行)
│   └── error.rs              # 错误类型 (74 行)
├── tests/
│   └── integration_test.rs   # 集成测试 (258 行)
└── docs/                      # 文档 (中英文双语)
```

**总计**: 2,027 行 Rust 代码 | 10 个测试 (100% 通过)

### 构建

```bash
# 开发构建
cargo check

# 运行测试
cargo test

# 发布构建
cargo build --release

# 静态二进制 (musl)
cargo build --release --target x86_64-unknown-linux-musl
```

### 许可证

MIT OR Apache-2.0
