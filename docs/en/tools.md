# Tool System Guide

Complete guide to ulnclaw's tool registry system.

## Table of Contents

- [Overview](#overview)
- [Tool Architecture](#tool-architecture)
- [Creating Tools](#creating-tools)
- [Tool Registry](#tool-registry)
- [Toolsets](#toolsets)
- [Advanced Patterns](#advanced-patterns)
- [Best Practices](#best-practices)
- [Examples](#examples)

## Overview

ulnclaw's tool system is inspired by Hermes Agent's registry pattern. It provides:

- **Self-Registering Tools**: Tools register themselves at module load time
- **JSON Schema Validation**: Tools define their parameters using JSON Schema
- **Async Handlers**: Tool handlers are async functions
- **Toolset Management**: Group related tools into toolsets
- **Dynamic Dispatch**: Agent dispatches tool calls by name

## Tool Architecture

### Core Types

```rust
// Tool definition exposed to the model
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,  // JSON Schema
}

// Registered tool with handler
pub struct Tool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub toolset: String,
    pub dangerous: bool,
}

// Async handler type
pub type ToolHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// Tool execution result
pub struct ToolResult {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}
```

### Flow

```
1. Model generates tool call
   ↓
2. Agent extracts tool name and arguments
   ↓
3. ToolRegistry.dispatch(name, args)
   ├─ Lookup tool by name
   ├─ Check if toolset is enabled
   ├─ Call handler with arguments
   └─ Return result
   ↓
4. Agent adds result to conversation
   ↓
5. Model processes result and continues
```

## Creating Tools

### Basic Tool

```rust
use ulnclaw::prelude::*;

let tool = tool("get_time")
    .description("Get current time")
    .parameters(json!({
        "type": "object",
        "properties": {}
    }))
    .handler(|_args| async {
        let now = chrono::Utc::now();
        Ok(json!({"time": now.to_rfc3339()}))
    })
    .build()?;

registry.register(tool);
```

### Tool with Parameters

```rust
tool("calculate")
    .description("Perform arithmetic operations")
    .parameters(json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "enum": ["add", "subtract", "multiply", "divide"]
            },
            "a": {"type": "number"},
            "b": {"type": "number"}
        },
        "required": ["operation", "a", "b"]
    }))
    .handler(|args| async move {
        let op = args["operation"].as_str().unwrap();
        let a = args["a"].as_f64().unwrap();
        let b = args["b"].as_f64().unwrap();
        
        let result = match op {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" => {
                if b == 0.0 {
                    return Err(ulnclaw::AgentError::tool("Division by zero"));
                }
                a / b
            }
            _ => return Err(ulnclaw::AgentError::tool("Unknown operation")),
        };
        
        Ok(json!({"result": result}))
    })
    .build()?
```

### Tool with Validation

```rust
tool("send_email")
    .description("Send an email message")
    .parameters(json!({
        "type": "object",
        "properties": {
            "to": {"type": "string", "format": "email"},
            "subject": {"type": "string", "maxLength": 100},
            "body": {"type": "string"}
        },
        "required": ["to", "subject", "body"]
    }))
    .handler(|args| async move {
        let to = args["to"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("to is required"))?;
        let subject = args["subject"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("subject is required"))?;
        let body = args["body"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("body is required"))?;
        
        // Validate email format
        if !to.contains('@') {
            return Err(ulnclaw::AgentError::tool("Invalid email format"));
        }
        
        // Validate subject length
        if subject.len() > 100 {
            return Err(ulnclaw::AgentError::tool("Subject too long"));
        }
        
        // Send email (implementation)
        send_email_impl(to, subject, body).await?;
        
        Ok(json!({
            "success": true,
            "message_id": "msg-123"
        }))
    })
    .build()?
```

### Dangerous Tool

Mark tools that require user confirmation:

```rust
tool("delete_database")
    .description("Delete an entire database")
    .parameters(json!({
        "type": "object",
        "properties": {
            "database": {"type": "string"}
        },
        "required": ["database"]
    }))
    .handler(|args| async move {
        let db = args["database"].as_str().unwrap();
        // Delete database
        Ok(json!({"deleted": db}))
    })
    .dangerous(true)  // Mark as dangerous
    .build()?
```

The agent can check `tool.dangerous` and prompt for confirmation before execution.

## Tool Registry

### Creating a Registry

```rust
let mut registry = ToolRegistry::new();
```

### Registering Tools

```rust
registry.register(tool);
```

### Dispatching Tools

```rust
let result = registry.dispatch("get_time", json!({})).await?;
```

### Querying Tools

```rust
// Check if tool exists
if registry.has("get_time") {
    println!("Tool registered");
}

// Get tool count
println!("{} tools registered", registry.len());

// Get all tool names
let names = registry.names();

// Get tool definitions (for sending to model)
let definitions = registry.definitions();
```

### Unregistering Tools

```rust
if let Some(tool) = registry.unregister("old_tool") {
    println!("Unregistered: {}", tool.definition.name);
}
```

## Toolsets

Toolsets group related tools for easy management.

### Defining Toolsets

```rust
tool("query_database")
    .description("Query a database")
    .handler(|args| async { /* ... */ })
    .toolset("database")  // Assign to toolset
    .build()?

tool("list_tables")
    .description("List database tables")
    .handler(|args| async { /* ... */ })
    .toolset("database")  // Same toolset
    .build()?
```

### Managing Toolsets

```rust
// Get all toolset names
let toolsets = registry.toolset_names();

// Get tools in a toolset
let db_tools = registry.toolset_tools("database");

// Disable a toolset
registry.disable_toolset("database");

// Enable a toolset
registry.enable_toolset("database");
```

### Toolset-Based Filtering

```rust
// Only send enabled tools to model
let definitions = registry.definitions();  // Excludes disabled toolsets

// Check if toolset is available
if !registry.toolset_names().contains(&"database".to_string()) {
    println!("Database tools not available");
}
```

## Advanced Patterns

### Tool with State

Use closures to capture state:

```rust
let counter = Arc::new(Mutex::new(0));
let counter_clone = counter.clone();

let tool = tool("increment")
    .description("Increment counter")
    .handler(move |_args| {
        let counter = counter_clone.clone();
        async move {
            let mut count = counter.lock().unwrap();
            *count += 1;
            Ok(json!({"count": *count}))
        }
    })
    .build()?;
```

### Tool with External Dependencies

```rust
use reqwest::Client;

let client = Arc::new(Client::new());
let client_clone = client.clone();

tool("fetch_url")
    .description("Fetch content from URL")
    .parameters(json!({
        "type": "object",
        "properties": {
            "url": {"type": "string"}
        },
        "required": ["url"]
    }))
    .handler(move |args| {
        let client = client_clone.clone();
        async move {
            let url = args["url"].as_str().unwrap();
            let response = client.get(url).send().await?;
            let body = response.text().await?;
            Ok(json!({"body": body}))
        }
    })
    .build()?
```

### Tool Composition

Create higher-level tools from lower-level ones:

```rust
// Low-level tool
let registry = Arc::new(Mutex::new(ToolRegistry::new()));
registry.lock().unwrap().register(
    tool("http_get")
        .handler(|args| async { /* ... */ })
        .build()?
);

// High-level tool using low-level
let registry_clone = registry.clone();
let tool = tool("check_api_status")
    .description("Check if API is healthy")
    .handler(move |args| {
        let registry = registry_clone.clone();
        async move {
            let result = registry.lock().unwrap()
                .dispatch("http_get", json!({"url": "https://api.example.com/health"}))
                .await?;
            
            let status = result["status"].as_u64().unwrap_or(0);
            Ok(json!({"healthy": status == 200}))
        }
    })
    .build()?;
```

### Dynamic Tool Registration

Register tools based on configuration:

```rust
fn register_tools_from_config(registry: &mut ToolRegistry, config: &Config) -> Result<()> {
    for tool_config in &config.tools {
        let tool = create_tool_from_config(tool_config)?;
        registry.register(tool);
    }
    Ok(())
}
```

## Best Practices

### 1. Clear Descriptions

```rust
// ❌ Bad
tool("calc")
    .description("Do math")
    .build()?

// ✅ Good
tool("calculate")
    .description("Perform arithmetic operations (add, subtract, multiply, divide) on two numbers")
    .build()?
```

### 2. Detailed Parameter Schemas

```rust
// ❌ Bad
tool("query")
    .parameters(json!({
        "type": "object",
        "properties": {
            "q": {"type": "string"}
        }
    }))
    .build()?

// ✅ Good
tool("search_database")
    .parameters(json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query string"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of results",
                "default": 10,
                "minimum": 1,
                "maximum": 100
            },
            "sort_by": {
                "type": "string",
                "description": "Sort field",
                "enum": ["relevance", "date", "name"]
            }
        },
        "required": ["query"]
    }))
    .build()?
```

### 3. Input Validation

```rust
tool("create_user")
    .handler(|args| async move {
        let username = args["username"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("username is required"))?;
        
        // Validate username
        if username.len() < 3 {
            return Err(ulnclaw::AgentError::tool("Username must be at least 3 characters"));
        }
        
        if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(ulnclaw::AgentError::tool("Username must be alphanumeric"));
        }
        
        // Create user
        Ok(json!({"user_id": "user-123"}))
    })
    .build()?
```

### 4. Error Handling

```rust
tool("read_file")
    .handler(|args| async move {
        let path = args["path"].as_str().unwrap();
        
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(json!({"content": content})),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(ulnclaw::AgentError::tool(format!("File not found: {}", path)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(ulnclaw::AgentError::tool(format!("Permission denied: {}", path)))
            }
            Err(e) => {
                Err(ulnclaw::AgentError::tool(format!("Failed to read file: {}", e)))
            }
        }
    })
    .build()?
```

### 5. Structured Responses

```rust
tool("list_files")
    .handler(|args| async move {
        let path = args["path"].as_str().unwrap();
        let entries = std::fs::read_dir(path)?;
        
        let files: Vec<Value> = entries
            .filter_map(|e| e.ok())
            .map(|e| {
                json!({
                    "name": e.file_name().to_string_lossy(),
                    "is_dir": e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                    "size": e.metadata().map(|m| m.len()).unwrap_or(0)
                })
            })
            .collect();
        
        Ok(json!({
            "path": path,
            "count": files.len(),
            "files": files
        }))
    })
    .build()?
```

### 6. Idempotent Operations

```rust
tool("set_config")
    .description("Set configuration value (idempotent)")
    .handler(|args| async move {
        let key = args["key"].as_str().unwrap();
        let value = args["value"].as_str().unwrap();
        
        // Setting the same value multiple times has the same effect
        set_config_impl(key, value).await?;
        
        Ok(json!({"key": key, "value": value, "updated": true}))
    })
    .build()?
```

## Examples

### Complete Tool Set

```rust
use ulnclaw::prelude::*;

fn create_filesystem_tools() -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    
    // Read file
    registry.register(tool("read_file")
        .description("Read contents of a file")
        .parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }))
        .handler(|args| async move {
            let path = args["path"].as_str().unwrap();
            let content = std::fs::read_to_string(path)?;
            Ok(json!({"content": content, "size": content.len()}))
        })
        .toolset("filesystem")
        .build()?);
    
    // Write file
    registry.register(tool("write_file")
        .description("Write content to a file")
        .parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        }))
        .handler(|args| async move {
            let path = args["path"].as_str().unwrap();
            let content = args["content"].as_str().unwrap();
            std::fs::write(path, content)?;
            Ok(json!({"written": path, "bytes": content.len()}))
        })
        .toolset("filesystem")
        .dangerous(true)
        .build()?);
    
    // List directory
    registry.register(tool("list_directory")
        .description("List files in a directory")
        .parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }))
        .handler(|args| async move {
            let path = args["path"].as_str().unwrap();
            let entries: Vec<String> = std::fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            Ok(json!({"entries": entries, "count": entries.len()}))
        })
        .toolset("filesystem")
        .build()?);
    
    Ok(registry)
}
```

### Tool with Caching

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Cache {
    data: HashMap<String, (Value, Instant)>,
    ttl: Duration,
}

impl Cache {
    fn new(ttl: Duration) -> Self {
        Self {
            data: HashMap::new(),
            ttl,
        }
    }
    
    fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key).and_then(|(value, time)| {
            if time.elapsed() < self.ttl {
                Some(value)
            } else {
                None
            }
        })
    }
    
    fn set(&mut self, key: String, value: Value) {
        self.data.insert(key, (value, Instant::now()));
    }
}

let cache = Arc::new(Mutex::new(Cache::new(Duration::from_secs(60))));
let cache_clone = cache.clone();

tool("get_weather")
    .description("Get weather (cached for 60 seconds)")
    .handler(move |args| {
        let cache = cache_clone.clone();
        async move {
            let city = args["city"].as_str().unwrap();
            
            // Check cache
            if let Some(cached) = cache.lock().unwrap().get(city) {
                return Ok(cached.clone());
            }
            
            // Fetch from API
            let weather = fetch_weather(city).await?;
            
            // Update cache
            cache.lock().unwrap().set(city.to_string(), weather.clone());
            
            Ok(weather)
        }
    })
    .build()?
```

### Tool with Rate Limiting

```rust
use std::time::{Duration, Instant};

struct RateLimiter {
    last_call: Option<Instant>,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(min_interval: Duration) -> Self {
        Self {
            last_call: None,
            min_interval,
        }
    }
    
    fn check(&mut self) -> Result<()> {
        if let Some(last) = self.last_call {
            let elapsed = last.elapsed();
            if elapsed < self.min_interval {
                return Err(ulnclaw::AgentError::tool(format!(
                    "Rate limited. Wait {}ms",
                    (self.min_interval - elapsed).as_millis()
                )));
            }
        }
        self.last_call = Some(Instant::now());
        Ok(())
    }
}

let limiter = Arc::new(Mutex::new(RateLimiter::new(Duration::from_secs(1))));
let limiter_clone = limiter.clone();

tool("expensive_api")
    .description("Call expensive API (rate limited)")
    .handler(move |args| {
        let limiter = limiter_clone.clone();
        async move {
            limiter.lock().unwrap().check()?;
            
            // Make API call
            let result = call_expensive_api(args).await?;
            Ok(result)
        }
    })
    .build()?
```

## Next Steps

- Read [Provider System Guide](providers.md) for implementing providers
- Check [API Reference](api-reference.md) for complete type documentation
- See [Integration Guide](integration.md) for using tools in applications
