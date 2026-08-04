# Architecture Guide

ulnclaw is designed as a modular, high-performance AI agent engine with clear separation of concerns.

## System Overview

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

## Core Components

### 1. Agent Module (`agent/mod.rs`)

The heart of ulnclaw, implementing the conversation loop that orchestrates all components.

**Key Types:**
- `Agent` - Main orchestrator struct
- `AgentConfig` - Configuration (max iterations, temperature, etc.)
- `RunResult` - Output containing content, history, usage stats
- `AgentCallbacks` - Event hooks for UI integration

**Conversation Loop:**
```
User Message → Build Prompt → Call Provider → 
  ├─ Tool Calls → Execute Tools → Append Results → Loop
  └─ Final Response → Return to User
```

**Design Decisions:**
- **Iteration Limits**: Prevents infinite loops (default: 50 iterations)
- **Tool Execution**: Sequential by default, concurrent option available
- **Thinking Blocks**: Automatically strips `<think>...</think>` tags from responses
- **Error Recovery**: Graceful degradation when tools fail

### 2. Provider Module (`provider/`)

Abstract interface for AI model backends with OpenAI-compatible implementation.

**Key Types:**
- `Provider` - Trait defining the provider interface
- `OpenAiProvider` - OpenAI-compatible implementation
- `Message`, `ToolCall`, `Usage` - Core conversation types
- `ProviderConfig` - Dynamic provider instantiation

**Supported Providers:**
- OpenAI (GPT-4, GPT-3.5)
- Anthropic (Claude, via OpenAI-compatible format)
- DashScope (Alibaba Qwen models)
- OpenRouter (multi-model gateway)
- Ollama (local models)
- llama.cpp (local server)

**Request/Response Flow:**
```
ProviderRequest {
  messages: Vec<Message>,
  tools: Vec<ToolDefinition>,
  model: String,
  max_tokens: Option<u32>,
  temperature: Option<f32>,
}

↓ HTTP POST to /v1/chat/completions

ProviderResponse {
  content: Option<String>,
  tool_calls: Vec<ToolCall>,
  usage: Option<Usage>,
  model: String,
  reasoning: Option<String>,
  finish_reason: Option<String>,
}
```

### 3. Tools Module (`tools/mod.rs`)

Self-registering tool system with JSON schema validation and async handlers.

**Key Types:**
- `ToolRegistry` - Central registry managing all tools
- `Tool` - Tool definition with handler
- `ToolBuilder` - Fluent API for tool creation
- `ToolDefinition` - JSON schema for model-facing descriptions

**Tool Registration Pattern:**
```rust
let tool = tool("get_weather")
    .description("Get current weather for a city")
    .parameters(json!({
        "type": "object",
        "properties": {
            "city": {"type": "string"}
        },
        "required": ["city"]
    }))
    .handler(|args| async move {
        let city = args["city"].as_str().unwrap();
        Ok(json!({"temp": "22°C", "condition": "sunny"}))
    })
    .toolset("weather")
    .build()?;

registry.register(tool);
```

**Toolset Management:**
- Group related tools into toolsets
- Enable/disable entire toolsets at runtime
- Query available toolsets and their tools

### 4. Session Module (`session/mod.rs`)

Conversation persistence with lineage tracking for compression scenarios.

**Key Types:**
- `SessionStore` - Trait for storage backends
- `MemorySessionStore` - In-memory implementation
- `Session` - Conversation state with metadata
- `SessionMetadata` - User ID, platform, model info

**Session Lifecycle:**
```
Create Session → Add Messages → Save → Load → Continue → Save
     │                                                    │
     └──────────────── Lineage Tracking ──────────────────┘
```

**Features:**
- Unique session IDs (UUID v4)
- Conversation history preservation
- Full-text search across sessions
- Metadata tracking (user, platform, model)
- Parent-child relationships for compressed sessions

### 5. Context Module (`context/mod.rs`)

Intelligent prompt building and context window management.

**Key Types:**
- `PromptBuilder` - Tiered system prompt assembly
- `ContextCompressor` - Context window optimization

**Prompt Building Strategy:**
```
System Prompt = Identity + Tool Guidance + Skills + 
                Context Files + Environment Hints + 
                Memory + Timestamp
```

**Tiered Assembly:**
1. **Stable Tier**: Identity, tool guidance, skills (rarely changes)
2. **Context Tier**: Context files, environment hints (per-session)
3. **Volatile Tier**: Memory, timestamp (per-request)

**Context Compression:**
- Estimates token count (rough: 4 chars per token)
- Detects when context exceeds limits
- Placeholder for future model-based summarization

### 6. Error Module (`error.rs`)

Comprehensive error types with automatic conversion.

**Error Categories:**
- `AgentError::Provider` - API failures, auth errors, rate limits
- `AgentError::Tool` - Tool execution failures
- `AgentError::ToolNotFound` - Missing tool in registry
- `AgentError::Session` - Persistence errors
- `AgentError::Context` - Context management errors
- `AgentError::Config` - Configuration errors
- `AgentError::IterationLimit` - Max iterations exceeded
- `AgentError::Http` - Network errors (from reqwest)
- `AgentError::Json` - Serialization errors
- `AgentError::Internal` - Generic internal errors

## Data Flow

### Complete Conversation Flow

```
1. User Input
   ↓
2. Agent::run()
   ├─ Load conversation history (optional)
   ├─ Add system prompt
   ├─ Add user message
   ↓
3. Conversation Loop (iteration 1..N)
   ├─ Build API request
   │   ├─ Serialize messages
   │   ├─ Collect tool definitions
   │   └─ Set model parameters
   ├─ Call Provider
   │   ├─ HTTP POST to API endpoint
   │   ├─ Parse response
   │   └─ Extract content/tool_calls
   ├─ Check for tool calls
   │   ├─ Yes: Execute tools
   │   │   ├─ Dispatch to handler
   │   │   ├─ Capture result
   │   │   ├─ Add to messages
   │   │   └─ Continue loop
   │   └─ No: Final response
   └─ Check iteration limit
   ↓
4. Return RunResult
   ├─ Final content
   ├─ Complete conversation history
   ├─ Token usage statistics
   ├─ Iteration count
   └─ Tool call records
```

## Design Principles

### 1. Prompt Stability
System prompt doesn't change mid-conversation. This enables:
- Efficient caching (Anthropic prefix caching)
- Consistent agent behavior
- Predictable token usage

### 2. Observable Execution
Every operation is visible through callbacks:
- `on_tool_start` - Before tool execution
- `on_tool_complete` - After tool execution
- `on_stream_delta` - Streaming text chunks
- `on_thinking` - Model thinking phase
- `on_step` - Each iteration complete

### 3. Interruptible
All async operations can be cancelled:
- HTTP requests via `tokio::select!`
- Tool execution via cancellation tokens
- Clean shutdown without resource leaks

### 4. Platform-Agnostic Core
One `Agent` serves multiple use cases:
- CLI applications
- Web services
- API servers
- Embedded systems

### 5. Loose Coupling
Optional subsystems use registry patterns:
- Tools register themselves, no hard dependencies
- Providers implement traits, swappable at runtime
- Sessions use trait objects, backend-agnostic

## Performance Considerations

### Memory Management
- **Arc for Providers**: Shared ownership across async tasks
- **Mutex for Registry**: Thread-safe tool access
- **Efficient Serialization**: serde_json with zero-copy where possible

### Concurrency
- **Async/Await**: Non-blocking I/O throughout
- **Concurrent Tool Execution**: Optional parallel tool calls
- **Tokio Runtime**: Configurable thread pool

### Network Optimization
- **Connection Pooling**: reqwest client reuse
- **Timeout Configuration**: 120s default, configurable
- **Error Retry**: Automatic retry with exponential backoff (planned)

## Extensibility Points

### Adding New Providers
1. Implement `Provider` trait
2. Add request/response types
3. Register in `ProviderConfig::build()`

### Adding New Tools
1. Use `ToolBuilder` fluent API
2. Define JSON schema
3. Implement async handler
4. Register with `ToolRegistry`

### Adding New Session Backends
1. Implement `SessionStore` trait
2. Add persistence logic
3. Swap in `Agent` constructor

### Custom Context Management
1. Extend `PromptBuilder` with new tiers
2. Implement compression algorithms
3. Hook into agent loop

## Security Considerations

### API Key Management
- Keys stored in `ProviderConfig`, not hardcoded
- Support for environment variables
- Never logged or exposed in errors

### Tool Execution Sandboxing
- Tools run in async context
- No direct filesystem access by default
- Handlers responsible for their own security

### Input Validation
- JSON schema validation for tool arguments
- Error types for invalid inputs
- Graceful degradation on malformed data

## Future Enhancements

### Planned Features
- **Streaming Responses**: Real-time token streaming
- **Context Compression**: Model-based conversation summarization
- **Subagent Delegation**: Hierarchical agent spawning
- **MCP Protocol**: Model Context Protocol support
- **SQLite Backend**: Production-ready session storage
- **Anthropic Native**: Direct Claude API support
- **Retry Logic**: Automatic retry with backoff
- **Metrics**: Prometheus/OpenTelemetry integration

### Architecture Evolution
- **Plugin System**: Dynamic tool/provider loading
- **Distributed Agents**: Multi-node agent coordination
- **Persistent Memory**: Long-term knowledge storage
- **Multi-modal**: Image/audio/video support

## References

- [Hermes Agent Architecture](https://github.com/NousResearch/hermes-agent)
- [OpenAI API Documentation](https://platform.openai.com/docs)
- [Rust Async Book](https://rust-lang.github.io/async-book/)
- [Tokio Documentation](https://tokio.rs/tokio/tutorial)
