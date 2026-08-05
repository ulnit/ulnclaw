//! ulnclaw - A Rust-based AI agent engine inspired by Hermes Agent
//!
//! # Overview
//!
//! ulnclaw provides a complete agent loop with tool calling, multi-provider
//! support, session persistence, and context management. It's designed to be
//! embedded into applications or used as a library.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Entry Points                              │
//! │  Agent::run()    Agent::chat()    Custom integration            │
//! └──────────┬──────────────┬───────────────────────┬───────────────┘
//!            │              │                       │
//!            ▼              ▼                       ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     Agent (conversation loop)                    │
//! │                                                                  │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
//! │  │ Prompt       │  │ Provider     │  │ Tool         │           │
//! │  │ Builder      │  │ Resolution   │  │ Dispatch     │           │
//! │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘           │
//! │         │                 │                 │                   │
//! │  ┌──────┴───────┐  ┌──────┴───────┐  ┌──────┴───────┐           │
//! │  │ Compression  │  │ OpenAI       │  │ Tool Registry│           │
//! │  │ & Caching    │  │ Compatible   │  │ (70+ tools)  │           │
//! │  └──────────────┘  └──────────────┘  └──────────────┘           │
//! └─────────┴─────────────────┴─────────────────┴───────────────────┘
//!            │                                    │
//!            ▼                                    ▼
//! ┌───────────────────┐              ┌──────────────────────┐
//! │ Session Storage   │              │ Tool Handlers         │
//! │ (In-memory/SQLite)│              │ Custom implementations│
//! └───────────────────┘              └──────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use ulnclaw::prelude::*;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Create provider
//!     let provider = OpenAiProvider::builder()
//!         .endpoint("https://api.openai.com")
//!         .api_key("sk-...")
//!         .model("gpt-4o")
//!         .build()?;
//!
//!     // Create tool registry
//!     let mut tools = ToolRegistry::new();
//!     tools.register(tool("get_time")
//!         .description("Get current time")
//!         .handler(|_args, _ctx| async move { Ok(json!({"time": "now"})) })
//!         .build()?);
//!
//!     // Create agent
//!     let agent = Agent::new(Arc::new(provider), tools)
//!         .with_config(AgentConfig {
//!             system_prompt: Some("You are a helpful assistant.".to_string()),
//!             ..Default::default()
//!         });
//!
//!     // Run
//!     let response = agent.chat("What time is it?").await?;
//!     println!("{}", response);
//!     Ok(())
//! }
//! ```

pub mod active_sessions;
pub mod agent;
pub mod ansi;
pub mod async_delegation;
pub mod backup;
pub mod banner;
pub mod binary_ext;
pub mod browser;
pub mod checkpoint;
pub mod config;
pub mod context;
pub mod cron;
pub mod desktop;
pub mod doctor;
pub mod env_guard;
pub mod env_probe;
pub mod environments;
pub mod curator;
pub mod error;
pub mod fallback;
pub mod gateway;
pub mod git_diff;
pub mod goals;
pub mod hermes_time;
pub mod insights;
pub mod mcp;
pub mod managed_gateway;
pub mod moa;
pub mod learning_graph;
pub mod learning_graph_render;
pub mod learning_mutations;
pub mod logs;
pub mod models_dev;
pub mod projects_db;
pub mod provider;
pub mod redact;
pub mod session;
pub mod skin;
pub mod skill_usage;
pub mod skills;
pub mod status;
pub mod think_scrubber;
pub mod tips;
pub mod title_generator;
pub mod tools;
pub mod toolsets;
pub mod update;
pub mod video_gen;
pub mod video_gen_backends;
pub mod video_gen_xai;
pub mod url_safety;

// Re-export core types for convenience
pub use agent::{Agent, AgentCallbacks, AgentConfig, RunResult, ToolCallRecord};
pub use context::{ContextCompressor, PromptBuilder};
pub use error::{AgentError, Result};
pub use provider::{
    FunctionCall, Message, Provider, ProviderConfig, ProviderKind, ProviderRequest,
    ProviderResponse, Role, ToolCall, Usage,
};
pub use provider::openai::OpenAiProvider;
pub use session::{MemorySessionStore, Session, SessionMetadata, SessionStore};
pub use session::sqlite::SqliteSessionStore;
pub use tools::{tool, Tool, ToolBuilder, ToolDefinition, ToolHandler, ToolRegistry, ToolResult};
pub use tools::context::ToolContext;
pub use tools::builtin::register_builtin_tools;
pub use config::UlncLawConfig;

/// Prelude module - convenient imports for common use cases
pub mod prelude {
    pub use crate::agent::{Agent, AgentConfig, RunResult};
    pub use crate::error::{AgentError, Result};
    pub use crate::provider::openai::OpenAiProvider;
    pub use crate::provider::{Message, Provider, Role};
    pub use crate::tools::{tool, ToolRegistry};
    pub use serde_json::json;
    pub use std::sync::Arc;
}

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// Get version string
pub fn version() -> &'static str {
    VERSION
}
