# Integration Guide

How to integrate ulnclaw into your Rust projects.

## Table of Contents

- [Basic Integration](#basic-integration)
- [Web Service Integration](#web-service-integration)
- [CLI Application](#cli-application)
- [Embedded Systems](#embedded-systems)
- [ZStep Integration](#zstep-integration)
- [Advanced Patterns](#advanced-patterns)
- [Best Practices](#best-practices)

## Basic Integration

### Step 1: Add Dependency

Add ulnclaw to your `Cargo.toml`:

```toml
[dependencies]
ulnclaw = { git = "https://gitee.com/ushaw/ulnclaw.git", branch = "master" }
tokio = { version = "1", features = ["full"] }
serde_json = "1.0"
```

Or use a local path for development:

```toml
[dependencies]
ulnclaw = { path = "../ulnclaw" }
```

### Step 2: Create Provider

```rust
use ulnclaw::prelude::*;

let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key("sk-...")  // Use environment variables in production
    .model("gpt-4")
    .build()?;
```

### Step 3: Define Tools

```rust
let mut tools = ToolRegistry::new();

// Simple tool
tools.register(tool("get_weather")
    .description("Get weather for a city")
    .parameters(json!({
        "type": "object",
        "properties": {
            "city": {"type": "string"}
        },
        "required": ["city"]
    }))
    .handler(|args| async move {
        let city = args["city"].as_str().unwrap_or("Unknown");
        Ok(json!({
            "city": city,
            "temperature": "22°C",
            "condition": "sunny"
        }))
    })
    .toolset("weather")
    .build()?);
```

### Step 4: Create and Run Agent

```rust
let agent = Agent::new(Arc::new(provider), tools)
    .with_config(AgentConfig {
        system_prompt: Some("You are a helpful weather assistant.".into()),
        max_iterations: 50,
        ..Default::default()
    });

let result = agent.run("What's the weather in Beijing?", None).await?;
println!("{}", result.content);
```

## Web Service Integration

### Built-in HTTP Gateway (Recommended)

ulnclaw ships an OpenAI-compatible gateway — no wrapper code needed. Any
OpenAI-compatible frontend (Open WebUI, LobeChat, LibreChat, NextChat,
ChatBox, ...) can connect by pointing at `http://host:port/v1`.

```bash
# config.toml
[gateway]
host = "127.0.0.1"
port = 8642
key = "sk-..."                 # optional bearer token (ULNCLAW_GATEWAY_KEY)

# run it
ulnclaw gateway
```

```rust
// ...or embed it in your own binary:
let router = ulnclaw::gateway::ApprovalRouter::new();
// Install an approve callback on the agent's tool context that routes
// confirm-tier commands into the run (see main.rs gateway_cmd for the
// full wiring), then:
let state = ulnclaw::gateway::GatewayState::new(
    agent,                       // Arc<Agent> with the SQLite store attached
    "my-agent".to_string(),      // advertised model name
    "openai".to_string(),        // provider label
    Some("sk-...".to_string()),  // bearer key (None = open)
    router,                      // run-approval router
)?;
ulnclaw::gateway::serve(state, "127.0.0.1", 8642).await?;
```

Endpoints: `/v1/chat/completions` (with `X-Ulnclaw-Session-Id` session
continuity), `/v1/responses`, `/v1/models`, `/v1/capabilities`,
`/api/sessions` CRUD + per-session chat, `/v1/runs` async runs with SSE
events and run-approval resolution (`POST /v1/runs/:id/approval`). See the
[API reference](api-reference.md#http-gateway-gateway) for the full table.

### Axum Example (Custom Routes)

```rust
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use ulnclaw::prelude::*;

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    session_id: Option<String>,
}

#[derive(Serialize)]
struct ChatResponse {
    reply: String,
    usage: Usage,
}

async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    let result = state.agent.run(&req.message, None).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Json(ChatResponse {
        reply: result.content,
        usage: result.usage,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key(&std::env::var("OPENAI_API_KEY").unwrap())
        .model("gpt-4")
        .build()?;

    let tools = ToolRegistry::new();
    let agent = Agent::new(Arc::new(provider), tools);

    let state = AppState {
        agent: Arc::new(agent),
    };

    let app = Router::new()
        .route("/chat", post(chat_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
    
    Ok(())
}
```

### With Session Persistence

```rust
use ulnclaw::session::{MemorySessionStore, SessionStore};

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    sessions: Arc<MemorySessionStore>,
}

async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    // Load or create session
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let history = state.sessions.load_session(&session_id)
        .ok()
        .flatten()
        .map(|s| s.messages);

    // Run agent
    let result = state.agent.run(&req.message, history).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Save session
    let mut session = ulnclaw::session::new_session(&session_id);
    session.messages = result.conversation.clone();
    state.sessions.save_session(&session).ok();

    Ok(Json(ChatResponse {
        reply: result.content,
        usage: result.usage,
    }))
}
```

## CLI Application

### Interactive REPL

```rust
use ulnclaw::prelude::*;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<()> {
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key(&std::env::var("OPENAI_API_KEY").unwrap())
        .model("gpt-4")
        .build()?;

    let tools = ToolRegistry::new();
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("You are a helpful assistant.".into()),
            ..Default::default()
        });

    println!("ulnclaw CLI - Type 'exit' to quit\n");

    let mut history = Vec::new();
    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        if input == "exit" {
            break;
        }

        match agent.run(input, Some(history.clone())).await {
            Ok(result) => {
                println!("\n{}\n", result.content);
                history = result.conversation;
            }
            Err(e) => {
                eprintln!("Error: {}\n", e);
            }
        }
    }

    Ok(())
}
```

## Embedded Systems

### Resource-Constrained Environments

```rust
use ulnclaw::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Use smaller model for embedded systems
    let provider = OpenAiProvider::builder()
        .endpoint("http://localhost:11434/v1")  // Local Ollama
        .model("qwen2.5:1.5b")  // Small model
        .max_tokens(1024)  // Limit tokens
        .build()?;

    // Minimal tools
    let mut tools = ToolRegistry::new();
    tools.register(tool("read_sensor")
        .description("Read sensor value")
        .handler(|args| async move {
            let sensor = args["sensor"].as_str().unwrap();
            // Read from hardware
            Ok(json!({"value": 42.0, "unit": "°C"}))
        })
        .build()?);

    // Conservative config
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            max_iterations: 10,  // Fewer iterations
            strip_thinking_blocks: true,
            ..Default::default()
        });

    let result = agent.run("Read temperature sensor", None).await?;
    println!("{}", result.content);

    Ok(())
}
```

## ZStep Integration

### Replacing Hand-Rolled Agent Loop

ulnclaw is designed to power the ZStep AI assistant. Here's how to integrate it:

#### Before (Hand-Rolled)

```rust
// Old approach in zstep-api/src/lib.rs
async fn agent_chat(
    State(state): State<ApiState>,
    Json(request): Json<AgentChatRequest>,
) -> Result<Json<AgentChatResponse>> {
    // Manual provider selection
    // Manual tool calling loop
    // Manual message formatting
    // Manual error handling
    // ...
}
```

#### After (ulnclaw)

```rust
use ulnclaw::prelude::*;
use crate::agent_bridge::{build_agent, convert_history};

async fn agent_chat(
    State(state): State<ApiState>,
    Json(request): Json<AgentChatRequest>,
) -> Result<Json<AgentChatResponse>> {
    // Build provider from ZStep config
    let provider_config = get_provider_config(&state).await?;
    let provider = provider_config.build()?;

    // Build tools from ZStep MCP tools
    let tools = build_zstep_tools(&state).await;

    // Create agent
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some(get_system_prompt()),
            max_iterations: 50,
            ..Default::default()
        });

    // Convert history
    let history = request.conversation_id
        .and_then(|id| load_history(&state, &id))
        .map(|h| convert_history(&h));

    // Run agent
    let result = agent.run(&request.message, history).await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(AgentChatResponse {
        reply: result.content,
        tools_called: result.tool_calls.into_iter().map(|tc| {
            ToolCallInfo {
                name: tc.name,
                arguments: tc.arguments,
                result: tc.result,
            }
        }).collect(),
        usage: result.usage,
    }))
}
```

### Bridge Module

Create `src/agent_bridge.rs` in zstep-api:

```rust
use crate::ApiState;
use ulnclaw::{
    Agent, AgentConfig,
    provider::{Message, ProviderConfig, ProviderKind},
    tools::{tool, ToolRegistry},
};
use std::sync::Arc;

pub fn build_zstep_tools(state: &ApiState) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    
    // Register all ZStep MCP tools
    register_connection_tools(&mut registry, state);
    register_task_tools(&mut registry, state);
    register_run_tools(&mut registry, state);
    register_system_tools(&mut registry, state);
    
    registry
}

fn register_connection_tools(registry: &mut ToolRegistry, state: &ApiState) {
    // Use macro to create tools that call execute_agent_tool
    macro_rules! zstep_tool {
        ($name:expr, $desc:expr, $params:expr) => {{
            let state_clone = state.clone();
            tool($name)
                .description($desc)
                .parameters($params)
                .handler(move |args| {
                    let state = state_clone.clone();
                    let name = $name.to_string();
                    async move {
                        execute_agent_tool(&state, &name, &args).await
                            .map_err(|e| ulnclaw::AgentError::tool(e))
                    }
                })
                .build()
        }};
    }

    registry.register(zstep_tool!(
        "list_connections",
        "List all connection assets",
        json!({"type": "object", "properties": {}})
    ).unwrap());

    // ... more tools
}
```

## Advanced Patterns

### Streaming Responses

```rust
use futures::StreamExt;

let agent = Agent::new(Arc::new(provider), tools)
    .with_callbacks(AgentCallbacks {
        on_stream_delta: Some(Box::new(|delta| {
            print!("{}", delta);
            std::io::stdout().flush().unwrap();
        })),
        ..Default::default()
    });
```

### Tool Progress Tracking

```rust
let agent = Agent::new(Arc::new(provider), tools)
    .with_callbacks(AgentCallbacks {
        on_tool_start: Some(Box::new(|name, args| {
            println!("🔧 Executing: {}", name);
        })),
        on_tool_complete: Some(Box::new(|name, result| {
            println!("✓ Completed: {}", name);
        })),
        ..Default::default()
    });
```

### Multi-Provider Fallback

```rust
use ulnclaw::provider::ProviderConfig;

let providers = vec![
    ProviderConfig {
        name: "Primary".into(),
        kind: ProviderKind::OpenAiCompatible,
        endpoint: "https://api.openai.com/v1".into(),
        api_key: Some("sk-primary".into()),
        model: "gpt-4".into(),
        ..Default::default()
    },
    ProviderConfig {
        name: "Fallback".into(),
        kind: ProviderKind::OpenAiCompatible,
        endpoint: "https://dashscope.aliyuncs.com/compatible-mode".into(),
        api_key: Some("sk-fallback".into()),
        model: "qwen-plus".into(),
        ..Default::default()
    },
];

let mut last_error = None;
for config in providers {
    match config.build() {
        Ok(provider) => {
            let agent = Agent::new(Arc::from(provider), tools.clone());
            match agent.run(&message, None).await {
                Ok(result) => return Ok(result),
                Err(e) => last_error = Some(e),
            }
        }
        Err(e) => last_error = Some(e),
    }
}

Err(last_error.unwrap())
```

### Custom Context Management

```rust
use ulnclaw::context::PromptBuilder;

let system_prompt = PromptBuilder::new()
    .identity("You are ZStep AI, a data synchronization assistant.")
    .tool_guidance("Use tools to manage connections, tasks, and runs.")
    .add_skill("Always confirm before destructive operations.")
    .add_context_file(include_str!("zstep_context.md"))
    .add_env_hint("platform", "ZStep v1.0")
    .memory("User is an experienced data engineer.")
    .build();

let agent = Agent::new(Arc::new(provider), tools)
    .with_config(AgentConfig {
        system_prompt: Some(system_prompt),
        ..Default::default()
    });
```

## Best Practices

### 1. Environment Variables for API Keys

```rust
// ❌ Don't hardcode keys
let provider = OpenAiProvider::builder()
    .api_key("sk-...")
    .build()?;

// ✅ Use environment variables
let provider = OpenAiProvider::builder()
    .api_key(&std::env::var("OPENAI_API_KEY")?)
    .build()?;
```

### 2. Error Handling

```rust
// ✅ Handle errors gracefully
match agent.run(&message, None).await {
    Ok(result) => {
        println!("{}", result.content);
        log_usage(&result.usage);
    }
    Err(ulnclaw::AgentError::Provider(msg)) => {
        eprintln!("Provider error: {}", msg);
        // Retry or fallback
    }
    Err(ulnclaw::AgentError::IterationLimit(n)) => {
        eprintln!("Hit iteration limit: {}", n);
        // Simplify task or increase limit
    }
    Err(e) => {
        eprintln!("Unexpected error: {}", e);
        // Log and alert
    }
}
```

### 3. Tool Design

```rust
// ✅ Clear descriptions and schemas
tool("query_database")
    .description("Execute a read-only SQL query on the specified database")
    .parameters(json!({
        "type": "object",
        "properties": {
            "database": {
                "type": "string",
                "description": "Database name"
            },
            "query": {
                "type": "string",
                "description": "SQL SELECT query"
            }
        },
        "required": ["database", "query"]
    }))
    .handler(|args| async move {
        // Validate inputs
        let db = args["database"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("database is required"))?;
        let query = args["query"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("query is required"))?;
        
        // Execute safely
        let result = execute_query(db, query).await?;
        Ok(json!({"rows": result, "count": result.len()}))
    })
    .build()?
```

### 4. Session Management

```rust
// ✅ Persist sessions for multi-turn conversations
let session_store = MemorySessionStore::new();

// First message
let result1 = agent.run("Hello", None).await?;
let mut session = ulnclaw::session::new_session("user-123");
session.messages = result1.conversation;
session_store.save_session(&session)?;

// Second message (with history)
let history = session_store.load_session("user-123")?
    .map(|s| s.messages);
let result2 = agent.run("How are you?", history).await?;
```

### 5. Monitoring and Logging

```rust
use tracing::{info, warn, error};

let agent = Agent::new(Arc::new(provider), tools)
    .with_callbacks(AgentCallbacks {
        on_tool_start: Some(Box::new(|name, args| {
            info!(tool = name, args = %args, "Tool execution started");
        })),
        on_tool_complete: Some(Box::new(|name, result| {
            info!(tool = name, "Tool execution completed");
        })),
        on_step: Some(Box::new(|iteration| {
            info!(iteration = iteration, "Agent iteration completed");
        })),
        ..Default::default()
    });
```

### 6. Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ulnclaw::prelude::*;

    #[tokio::test]
    async fn test_tool_registration() {
        let mut registry = ToolRegistry::new();
        
        registry.register(tool("test_tool")
            .description("Test tool")
            .handler(|_| async { Ok(json!({"success": true})) })
            .build()
            .unwrap());

        assert!(registry.has("test_tool"));
        assert_eq!(registry.len(), 1);
    }

    #[tokio::test]
    async fn test_tool_dispatch() {
        let mut registry = ToolRegistry::new();
        
        registry.register(tool("echo")
            .description("Echo input")
            .handler(|args| async move {
                Ok(json!({"echo": args}))
            })
            .build()
            .unwrap());

        let result = registry.dispatch("echo", json!({"msg": "hello"})).await.unwrap();
        assert_eq!(result["echo"]["msg"], "hello");
    }
}
```

## Troubleshooting

### Common Issues

**Issue**: "API error (401): Invalid API key"
- **Solution**: Check API key is set correctly, use environment variables

**Issue**: "Tool not found: xxx"
- **Solution**: Ensure tool is registered with correct name (case-sensitive)

**Issue**: "Iteration limit exceeded: 50 iterations"
- **Solution**: Increase `max_iterations` in `AgentConfig` or simplify the task

**Issue**: "Provider error: Connection timeout"
- **Solution**: Check network connectivity, increase timeout in provider config

**Issue**: "JSON error: invalid type"
- **Solution**: Ensure tool handler returns valid JSON value

### Debug Mode

Enable detailed logging:

```rust
use tracing_subscriber;

tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

## Next Steps

- Read [Tool System Guide](tools.md) for advanced tool patterns
- Read [Provider System Guide](providers.md) for implementing custom providers
- Check [API Reference](api-reference.md) for complete type documentation
- See [Development Guide](development.md) for contributing to ulnclaw
