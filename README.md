# ulnclaw 🦞

A Rust-based AI agent engine inspired by [Hermes Agent](https://github.com/NousResearch/hermes-agent).

## Overview

ulnclaw provides a complete agent loop with tool calling, multi-provider support, session persistence, and context management. It's designed to be embedded into applications or used as a standalone library.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Entry Points                              │
│  Agent::run()    Agent::chat()    Custom integration            │
└──────────┬──────────────┬───────────────────────┬───────────────┘
           │              │                       │
           ▼              ▼                       ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Agent (conversation loop)                    │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │ Prompt       │  │ Provider     │  │ Tool         │           │
│  │ Builder      │  │ Resolution   │  │ Dispatch     │           │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘           │
│         │                 │                 │                   │
│  ┌──────┴───────┐  ┌──────┴───────┐  ┌──────┴───────┐           │
│  │ Compression  │  │ OpenAI       │  │ Tool Registry│           │
│  │ & Caching    │  │ Compatible   │  │ (dynamic)    │           │
│  └──────────────┘  └──────────────┘  └──────────────┘           │
└─────────┴─────────────────┴─────────────────┴───────────────────┘
           │                                    │
           ▼                                    ▼
┌───────────────────┐              ┌──────────────────────┐
│ Session Storage   │              │ Tool Handlers         │
│ (In-memory/SQLite)│              │ Custom implementations│
└───────────────────┘              └──────────────────────┘
```

## Features

- **Agent Loop**: Automatic tool calling loop until completion with configurable iteration limits
- **Tool Registry**: Self-registering tools with JSON schema, async handlers, and toolset management
- **Multi-Provider**: OpenAI-compatible endpoints (OpenAI, DashScope, OpenRouter, Ollama, llama.cpp)
- **Session Persistence**: In-memory and SQLite-based conversation storage with lineage tracking
- **Context Management**: Prompt builder with tiered assembly, context compression (planned)
- **Callbacks**: Progress, thinking, streaming, and step callbacks for UI integration
- **Error Handling**: Comprehensive error types with provider fallback support

## Quick Start

```rust
use ulnclaw::prelude::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // Create provider
    let provider = OpenAiProvider::builder()
        .endpoint("https://dashscope.aliyuncs.com/compatible-mode")
        .api_key("sk-...")
        .model("qwen-plus")
        .build()?;

    // Create tool registry
    let mut tools = ToolRegistry::new();
    tools.register(tool("get_time")
        .description("Get current time")
        .handler(|_args| async { 
            Ok(json!({"time": "2026-08-04 12:00:00"})) 
        })
        .toolset("system")
        .build()?);

    // Create agent
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("You are a helpful assistant.".to_string()),
            max_iterations: 50,
            ..Default::default()
        });

    // Run conversation
    let result = agent.run("What time is it?", None).await?;
    println!("Response: {}", result.content);
    println!("Tool calls: {}", result.tool_calls.len());
    println!("Iterations: {}", result.iterations);
    
    Ok(())
}
```

## Modules

### `agent` - Core conversation loop
- `Agent`: Main orchestrator with `run()` and `chat()` methods
- `AgentConfig`: Configuration (max iterations, temperature, system prompt)
- `AgentCallbacks`: Event hooks (tool_start, tool_complete, stream_delta)
- `RunResult`: Output with content, conversation history, usage, and tool call records

### `provider` - AI model backends
- `Provider` trait: Async interface for chat completions
- `OpenAiProvider`: OpenAI-compatible API client (works with DashScope, Ollama, llama.cpp)
- `ProviderConfig`: Dynamic provider instantiation from config
- `Message`, `ToolCall`, `Usage`: Core conversation types

### `tools` - Tool registry
- `ToolRegistry`: Central registry with dispatch, toolset management
- `ToolBuilder`: Fluent API for tool creation
- `ToolDefinition`: JSON schema for model-facing tool descriptions
- `ToolHandler`: Async function type for tool execution

### `session` - Conversation persistence
- `SessionStore` trait: Abstraction for storage backends
- `MemorySessionStore`: In-memory implementation
- `Session`: Conversation state with lineage tracking

### `context` - Prompt management
- `PromptBuilder`: Tiered system prompt assembly (identity → tools → context → memory)
- `ContextCompressor`: Context window optimization (planned)

## Provider Support

| Provider | Status | Notes |
|----------|--------|-------|
| OpenAI | ✅ | GPT-4o, GPT-4, etc. |
| DashScope (Alibaba) | ✅ | Qwen models |
| OpenRouter | ✅ | Multi-model gateway |
| Ollama | ✅ | Local models |
| llama.cpp | ✅ | Local server |
| Anthropic | 🔧 | Via OpenAI-compatible format |

## Design Principles

Inspired by Hermes Agent's design principles:

| Principle | What it means |
|-----------|---------------|
| **Prompt stability** | System prompt doesn't change mid-conversation |
| **Observable execution** | Every tool call visible via callbacks |
| **Interruptible** | API calls and tool execution can be cancelled |
| **Platform-agnostic core** | One Agent serves CLI, web, API, and embedded use cases |
| **Loose coupling** | Optional subsystems use registry patterns, not hard dependencies |

## Comparison with Hermes Agent

| Feature | Hermes Agent (Python) | ulnclaw (Rust) |
|---------|----------------------|----------------|
| Language | Python 3.11+ | Rust 2021 |
| Tool count | 70+ tools, 28 toolsets | Dynamic (user-defined) |
| API modes | 3 (chat_completions, codex, anthropic) | 1 (OpenAI-compatible) |
| Providers | 18+ | OpenAI-compatible (covers most) |
| Session storage | SQLite + FTS5 | In-memory + SQLite (planned) |
| Gateway | 25+ platform adapters | Not included (embed in host app) |
| Plugin system | Full plugin ecosystem | Tool registry only |
| Context compression | Lossy summarization | Planned |
| Subagent delegation | Yes | Planned |
| Streaming | Yes | Planned |

## Integration with ZStep

ulnclaw is designed to power the ZStep AI assistant. Integration replaces the hand-rolled
agent loop in `zstep-api` with the structured ulnclaw engine:

```rust
// In zstep-api: replace agent_chat handler
use ulnclaw::prelude::*;

let provider = /* from ZStep AI provider config */;
let tools = /* register all ZStep MCP tools */;
let agent = Agent::new(provider, tools).with_config(config);
let result = agent.run(&user_message, history).await?;
```

## Building

```bash
# Check
cargo check

# Test
cargo test

# Build
cargo build --release

# Build for musl (static binary)
cargo build --release --target x86_64-unknown-linux-musl
```

## License

MIT OR Apache-2.0
