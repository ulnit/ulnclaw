# Development Guide

Guide for contributing to and extending ulnclaw.

## Table of Contents

- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Building and Testing](#building-and-testing)
- [Adding New Providers](#adding-new-providers)
- [Adding New Tools](#adding-new-tools)
- [Adding New Session Backends](#adding-new-session-backends)
- [Code Style](#code-style)
- [Testing Guidelines](#testing-guidelines)
- [Documentation](#documentation)
- [Contributing](#contributing)

## Development Setup

### Prerequisites

- Rust 1.70+ (2021 edition)
- Cargo
- Git

### Clone and Setup

```bash
# Clone the repository
git clone https://gitee.com/ushaw/ulnclaw.git
cd ulnclaw

# Build the project
cargo build

# Run tests
cargo test

# Check for issues
cargo clippy
```

### IDE Setup

**VS Code / VSCodium:**
```bash
# Install rust-analyzer extension
code --install-extension rust-lang.rust-analyzer

# Install Even Better TOML
code --install-extension tamasfe.even-better-toml
```

**Recommended Settings** (`.vscode/settings.json`):
```json
{
    "rust-analyzer.checkOnSave.command": "clippy",
    "rust-analyzer.cargo.features": "all",
    "editor.formatOnSave": true
}
```

## Project Structure

```
ulnclaw/
├── Cargo.toml                 # Project manifest
├── Cargo.lock                 # Dependency lock file
├── README.md                  # Project documentation
├── src/
│   ├── lib.rs                # Library entry point
│   ├── main.rs               # CLI (chat/run/sessions/tools/skills/cron/gateway/init)
│   ├── error.rs              # Error types
│   ├── agent/                # Core agent loop (persistence, approval, delegation)
│   ├── provider/             # Provider trait + OpenAI-compatible implementation
│   ├── tools/
│   │   ├── mod.rs            # Tool registry & builder
│   │   ├── approval.rs       # Command approval policies
│   │   ├── context.rs        # ToolContext (session, store, callbacks)
│   │   └── builtin/          # 46 hermes tools (terminal, files, web, browser, ...)
│   ├── browser/              # CDP WebSocket client (browser_* tools)
│   ├── gateway/              # HTTP gateway (OpenAI-compatible API server)
│   ├── session/              # SessionStore trait + SQLite backend (FTS5)
│   ├── context/              # Prompt builder + context compression
│   ├── config/               # config.toml/.env/profiles + env overrides
│   ├── cron/                 # Schedule parsing, job store, scheduler
│   ├── skills/               # SKILL.md discovery & injection
│   ├── mcp/                  # MCP stdio client
│   └── toolsets.rs           # hermes-compatible toolset policy
├── tests/
│   └── integration_test.rs   # Integration tests
└── docs/                      # Documentation
    ├── en/                    # English docs
    └── zh/                    # Chinese docs
```

### Module Responsibilities

**lib.rs**
- Public API exports
- Prelude module for convenient imports
- Version information

**error.rs**
- `AgentError` enum with all error types
- Error conversion implementations
- Helper constructors

**agent/mod.rs**
- `Agent` struct - main orchestrator
- Conversation loop implementation
- Tool dispatch logic
- Callback system

**provider/mod.rs**
- `Provider` trait definition
- Core types: `Message`, `ToolCall`, `Usage`
- `ProviderConfig` for dynamic instantiation

**provider/openai.rs**
- `OpenAiProvider` implementation
- HTTP client configuration
- Request/response serialization

**tools/mod.rs**
- `ToolRegistry` - central registry
- `Tool` and `ToolDefinition` types
- `ToolBuilder` fluent API
- Toolset management

**session/mod.rs**
- `SessionStore` trait
- `MemorySessionStore` implementation
- Session lifecycle management

**context/mod.rs**
- `PromptBuilder` for system prompts
- `ContextCompressor` for optimization
- Token estimation

## Building and Testing

### Build Commands

```bash
# Development build
cargo build

# Release build
cargo build --release

# Check without building
cargo check

# Build for musl (static binary)
cargo build --release --target x86_64-unknown-linux-musl
```

### Test Commands

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_tool_registry

# Run tests in parallel
cargo test -- --test-threads=4

# Generate coverage report
cargo install cargo-tarpaulin
cargo tarpaulin --out Html
```

### Linting and Formatting

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Run clippy
cargo clippy

# Run clippy with all features
cargo clippy --all-features

# Fix clippy warnings automatically
cargo clippy --fix
```

### Documentation

```bash
# Generate docs
cargo doc --no-deps

# Open docs in browser
cargo doc --no-deps --open

# Check doc links
cargo doc --no-deps
# Then manually check for broken links
```

## Adding New Providers

### Step 1: Create Provider Module

Create `src/provider/your_provider.rs`:

```rust
use super::{Message, Provider, ProviderRequest, ProviderResponse, ToolCall, Usage};
use crate::error::{AgentError, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct YourProvider {
    client: Client,
    endpoint: String,
    api_key: Option<String>,
    model: String,
    name: String,
}

impl YourProvider {
    pub fn builder() -> YourProviderBuilder {
        YourProviderBuilder::default()
    }
}

pub struct YourProviderBuilder {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    name: Option<String>,
}

impl Default for YourProviderBuilder {
    fn default() -> Self {
        Self {
            endpoint: None,
            api_key: None,
            model: None,
            name: None,
        }
    }
}

impl YourProviderBuilder {
    pub fn endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = Some(endpoint.to_string());
        self
    }

    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = Some(key.to_string());
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn build(self) -> Result<YourProvider> {
        let endpoint = self.endpoint
            .ok_or_else(|| AgentError::config("endpoint is required"))?;
        let model = self.model
            .ok_or_else(|| AgentError::config("model is required"))?;
        let name = self.name.unwrap_or_else(|| model.clone());

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AgentError::Internal(format!("HTTP client error: {}", e)))?;

        Ok(YourProvider {
            client,
            endpoint,
            api_key: self.api_key,
            model,
            name,
        })
    }
}

#[async_trait]
impl Provider for YourProvider {
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        // Implement API call
        // 1. Convert request to API format
        // 2. Make HTTP request
        // 3. Parse response
        // 4. Convert to ProviderResponse
        
        todo!("Implement your provider")
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn name(&self) -> &str {
        &self.name
    }
}
```

### Step 2: Export Provider

Update `src/provider/mod.rs`:

```rust
pub mod openai;
pub mod your_provider;  // Add this

pub use your_provider::YourProvider;  // Add this
```

### Step 3: Add to ProviderConfig

Update `ProviderConfig::build()` in `src/provider/mod.rs`:

```rust
impl ProviderConfig {
    pub fn build(&self) -> Result<Box<dyn Provider>> {
        match self.kind {
            ProviderKind::OpenAiCompatible | ProviderKind::Ollama | ProviderKind::LlamaCpp => {
                // ... existing code
            }
            ProviderKind::YourProvider => {  // Add this
                let mut builder = your_provider::YourProvider::builder()
                    .endpoint(&self.endpoint)
                    .model(&self.model)
                    .name(&self.name);

                if let Some(ref key) = self.api_key {
                    builder = builder.api_key(key);
                }

                Ok(Box::new(builder.build()?))
            }
            // ...
        }
    }
}
```

### Step 4: Add ProviderKind Variant

Update `ProviderKind` enum:

```rust
pub enum ProviderKind {
    OpenAiCompatible,
    Ollama,
    LlamaCpp,
    Anthropic,
    Local,
    YourProvider,  // Add this
}
```

### Step 5: Write Tests

Create `tests/your_provider_test.rs`:

```rust
use ulnclaw::provider::YourProvider;

#[tokio::test]
async fn test_your_provider_builder() {
    let provider = YourProvider::builder()
        .endpoint("https://api.example.com")
        .model("test-model")
        .build();
    
    assert!(provider.is_ok());
}

#[tokio::test]
async fn test_your_provider_missing_endpoint() {
    let provider = YourProvider::builder()
        .model("test-model")
        .build();
    
    assert!(provider.is_err());
}
```

## Adding New Tools

### Simple Tool

```rust
use ulnclaw::prelude::*;

let tool = tool("greet")
    .description("Greet a person by name")
    .parameters(json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"}
        },
        "required": ["name"]
    }))
    .handler(|args| async move {
        let name = args["name"].as_str().unwrap_or("stranger");
        Ok(json!({"greeting": format!("Hello, {}!", name)}))
    })
    .toolset("social")
    .build()?;

registry.register(tool);
```

### Complex Tool with Validation

```rust
tool("calculate")
    .description("Evaluate mathematical expressions")
    .parameters(json!({
        "type": "object",
        "properties": {
            "expression": {"type": "string"},
            "precision": {"type": "integer", "default": 2}
        },
        "required": ["expression"]
    }))
    .handler(|args| async move {
        let expr = args["expression"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("expression must be a string"))?;
        
        let precision = args["precision"].as_u64().unwrap_or(2) as usize;
        
        // Validate expression (whitelist allowed characters)
        if !expr.chars().all(|c| c.is_digit(10) || "+-*/(). ".contains(c)) {
            return Err(ulnclaw::AgentError::tool("Invalid characters in expression"));
        }
        
        // Evaluate (use a safe evaluator)
        let result = evaluate_expression(expr)?;
        
        Ok(json!({
            "expression": expr,
            "result": result,
            "precision": precision
        }))
    })
    .build()?
```

### Dangerous Tool (Requires Confirmation)

```rust
tool("delete_file")
    .description("Delete a file from the filesystem")
    .parameters(json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"}
        },
        "required": ["path"]
    }))
    .handler(|args| async move {
        let path = args["path"].as_str().unwrap();
        std::fs::remove_file(path)?;
        Ok(json!({"deleted": path}))
    })
    .dangerous(true)  // Mark as dangerous
    .build()?
```

### Tool with External Dependencies

```rust
tool("http_request")
    .description("Make HTTP requests")
    .parameters(json!({
        "type": "object",
        "properties": {
            "url": {"type": "string"},
            "method": {"type": "string", "enum": ["GET", "POST", "PUT", "DELETE"]}
        },
        "required": ["url"]
    }))
    .handler(|args| async move {
        let url = args["url"].as_str().unwrap();
        let method = args["method"].as_str().unwrap_or("GET");
        
        let client = reqwest::Client::new();
        let response = match method {
            "GET" => client.get(url).send().await?,
            "POST" => client.post(url).send().await?,
            // ...
            _ => return Err(ulnclaw::AgentError::tool("Unsupported method")),
        };
        
        let status = response.status().as_u16();
        let body = response.text().await?;
        
        Ok(json!({
            "status": status,
            "body": body
        }))
    })
    .build()?
```

## Adding New Session Backends

### SQLite Backend Example

Create `src/session/sqlite.rs`:

```rust
use super::{Session, SessionStore};
use crate::error::{AgentError, Result};
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
}

impl SqliteSessionStore {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| AgentError::session(format!("Failed to open database: {}", e)))?;
        
        // Create table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                messages TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                parent_id TEXT,
                metadata TEXT NOT NULL
            )",
            [],
        ).map_err(|e| AgentError::session(format!("Failed to create table: {}", e)))?;
        
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save_session(&self, session: &Session) -> Result<()> {
        let conn = self.conn.lock()
            .map_err(|e| AgentError::session(format!("Lock error: {}", e)))?;
        
        let messages_json = serde_json::to_string(&session.messages)?;
        let metadata_json = serde_json::to_string(&session.metadata)?;
        
        conn.execute(
            "INSERT OR REPLACE INTO sessions 
             (id, conversation_id, messages, created_at_ms, updated_at_ms, parent_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.id,
                session.conversation_id,
                messages_json,
                session.created_at_ms,
                session.updated_at_ms,
                session.parent_id,
                metadata_json,
            ],
        ).map_err(|e| AgentError::session(format!("Failed to save session: {}", e)))?;
        
        Ok(())
    }

    fn load_session(&self, session_id: &str) -> Result<Option<Session>> {
        let conn = self.conn.lock()
            .map_err(|e| AgentError::session(format!("Lock error: {}", e)))?;
        
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, messages, created_at_ms, updated_at_ms, parent_id, metadata
             FROM sessions WHERE id = ?1"
        ).map_err(|e| AgentError::session(format!("Query error: {}", e)))?;
        
        let session = stmt.query_row(params![session_id], |row| {
            let messages_json: String = row.get(2)?;
            let metadata_json: String = row.get(6)?;
            
            Ok((
                row.get(0)?,
                row.get(1)?,
                messages_json,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                metadata_json,
            ))
        }).ok();
        
        if let Some((id, conv_id, messages_json, created, updated, parent, metadata_json)) = session {
            let messages = serde_json::from_str(&messages_json)?;
            let metadata = serde_json::from_str(&metadata_json)?;
            
            Ok(Some(Session {
                id,
                conversation_id: conv_id,
                messages,
                created_at_ms: created,
                updated_at_ms: updated,
                parent_id: parent,
                metadata,
            }))
        } else {
            Ok(None)
        }
    }

    // Implement other methods...
    fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        todo!()
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        todo!()
    }

    fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<Session>> {
        todo!()
    }
}
```

## Code Style

### Formatting

All code must be formatted with `cargo fmt`:

```bash
cargo fmt
```

### Naming Conventions

- **Types**: `PascalCase` (e.g., `AgentConfig`, `ToolRegistry`)
- **Functions/Methods**: `snake_case` (e.g., `chat_completion`, `load_session`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `MAX_ITERATIONS`)
- **Modules**: `snake_case` (e.g., `provider`, `tools`)

### Documentation

All public items must have doc comments:

```rust
/// Brief description of the function.
///
/// More detailed explanation if needed.
///
/// # Arguments
///
/// * `arg1` - Description of arg1
/// * `arg2` - Description of arg2
///
/// # Returns
///
/// Description of return value
///
/// # Examples
///
/// ```rust
/// let result = my_function(42);
/// assert_eq!(result, 84);
/// ```
pub fn my_function(arg1: i32, arg2: i32) -> i32 {
    arg1 + arg2
}
```

### Error Handling

Use `?` operator for error propagation:

```rust
// ✅ Good
fn read_file(path: &str) -> Result<String> {
    let content = std::fs::read_to_string(path)?;
    Ok(content)
}

// ❌ Bad
fn read_file(path: &str) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(e) => Err(AgentError::Internal(e.to_string())),
    }
}
```

### Async/Await

Prefer async/await over callbacks:

```rust
// ✅ Good
async fn fetch_data() -> Result<String> {
    let response = client.get(url).send().await?;
    let body = response.text().await?;
    Ok(body)
}

// ❌ Bad (callback-based)
fn fetch_data(callback: impl FnOnce(String)) {
    // ...
}
```

## Testing Guidelines

### Unit Tests

Place unit tests in the same file as the code:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_addition() {
        assert_eq!(add(2, 3), 5);
    }

    #[test]
    fn test_negative_numbers() {
        assert_eq!(add(-1, -1), -2);
    }
}
```

### Integration Tests

Place integration tests in `tests/` directory:

```rust
// tests/integration_test.rs
use ulnclaw::prelude::*;

#[tokio::test]
async fn test_full_conversation() {
    let provider = create_mock_provider();
    let tools = create_test_tools();
    let agent = Agent::new(Arc::new(provider), tools);
    
    let result = agent.run("Hello", None).await.unwrap();
    assert!(!result.content.is_empty());
}
```

### Mock Providers

Create mock providers for testing:

```rust
use async_trait::async_trait;

struct MockProvider;

#[async_trait]
impl Provider for MockProvider {
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        Ok(ProviderResponse {
            content: Some("Mock response".to_string()),
            tool_calls: vec![],
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            model: "mock".to_string(),
            reasoning: None,
            finish_reason: Some("stop".to_string()),
        })
    }

    fn model(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "MockProvider"
    }
}
```

## Documentation

### Writing Docs

All documentation should be:
- Clear and concise
- Include code examples
- Cover common use cases
- Mention pitfalls and edge cases

### Doc Structure

```
docs/
├── en/
│   ├── architecture.md    # System design
│   ├── api-reference.md   # Complete API docs
│   ├── integration.md     # Integration guide
│   ├── development.md     # This file
│   ├── tools.md          # Tool system guide
│   └── providers.md      # Provider system guide
└── zh/
    └── (Chinese translations)
```

### Building Docs

```bash
# Generate HTML docs
cargo doc --no-deps --open

# Check for broken links
cargo doc --no-deps
```

## Contributing

### Workflow

1. **Fork the repository**
2. **Create a feature branch**
   ```bash
   git checkout -b feature/my-feature
   ```
3. **Make changes**
   - Write code
   - Add tests
   - Update documentation
4. **Run checks**
   ```bash
   cargo fmt
   cargo clippy
   cargo test
   ```
5. **Commit changes**
   ```bash
   git commit -m "feat: add my feature"
   ```
6. **Push and create PR**
   ```bash
   git push origin feature/my-feature
   ```

### Commit Messages

Follow conventional commits:

- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `style:` - Code style changes
- `refactor:` - Code refactoring
- `test:` - Adding tests
- `chore:` - Maintenance tasks

**Examples:**
```
feat: add Anthropic provider support
fix: resolve tool dispatch race condition
docs: update integration guide with examples
refactor: simplify context compression logic
test: add unit tests for session persistence
```

### Code Review Checklist

- [ ] Code is formatted with `cargo fmt`
- [ ] No clippy warnings
- [ ] All tests pass
- [ ] New features have tests
- [ ] Documentation is updated
- [ ] Commit messages follow convention
- [ ] No breaking changes (or documented in CHANGELOG)

### Reporting Issues

When reporting issues, include:

- **ulnclaw version**
- **Rust version**
- **Operating system**
- **Minimal reproduction code**
- **Expected behavior**
- **Actual behavior**
- **Error messages/logs**

### Feature Requests

For feature requests, describe:

- **Use case** - What problem does this solve?
- **Proposed solution** - How should it work?
- **Alternatives considered** - What other approaches were tried?
- **Additional context** - Any relevant information

## Roadmap

### Planned Features

- [ ] Streaming responses
- [ ] Context compression
- [ ] Subagent delegation
- [ ] MCP protocol support
- [ ] SQLite session backend
- [ ] Anthropic native provider
- [ ] Automatic retry logic
- [ ] Metrics and tracing
- [ ] Plugin system
- [ ] Multi-modal support

### Contributing Ideas

- Implement missing features from roadmap
- Add more providers (Google Gemini, Cohere, etc.)
- Improve documentation
- Add examples
- Write blog posts
- Create video tutorials

## Getting Help

- **Documentation**: Read the docs in `docs/` directory
- **Issues**: Search existing issues on Gitee
- **Discussions**: Open a discussion on Gitee
- **Email**: Contact maintainers

## License

By contributing, you agree that your contributions will be licensed under the MIT OR Apache-2.0 License.
