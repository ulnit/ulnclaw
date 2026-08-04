//! Agent — the core conversation loop
//!
//! Port of hermes' AIAgent (run_agent.py): provider calls, tool dispatch with
//! approval gating, session persistence, context compression, delegation
//! (SubAgentRunner), cron execution (CronRunner), memory injection, and
//! callbacks.

use crate::error::{AgentError, Result};
use crate::provider::{Message, Provider, ProviderRequest, Role, Usage};
use crate::session::sqlite::SqliteSessionStore;
use crate::tools::approval::{classify_command, ApprovalDecision};
use crate::tools::context::{CronRunner, SubAgentRunner};
use crate::tools::ToolContext;
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Events surfaced to streaming consumers (gateway SSE).
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Content token delta from the model.
    Delta(String),
    /// Tool lifecycle progress (rendered as `hermes.tool.progress` SSE
    /// events so frontends can show activity without polluting history).
    ToolProgress { tool: String, status: String },
    /// Tool call about to run (Responses API `function_call` items).
    ToolStarted {
        name: String,
        call_id: String,
        arguments: String,
    },
    /// Tool call finished (Responses API `function_call_output` items).
    ToolCompleted { call_id: String, result: String },
}

tokio::task_local! {
    static STREAM_EMITTER: Arc<dyn Fn(StreamEvent) + Send + Sync>;
}

/// Emit an event to the active stream consumer, if any (task-local scope
/// installed by the gateway for streaming requests).
pub fn emit_stream_event(event: StreamEvent) {
    if let Ok(emitter) = STREAM_EMITTER.try_with(|e| e.clone()) {
        emitter(event);
    }
}

/// Run a future with a stream emitter installed (gateway streaming scope).
pub fn stream_scope<F: std::future::Future>(
    emitter: Arc<dyn Fn(StreamEvent) + Send + Sync>,
    future: F,
) -> tokio::task::futures::TaskLocalFuture<Arc<dyn Fn(StreamEvent) + Send + Sync>, F> {
    STREAM_EMITTER.scope(emitter, future)
}

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
    /// Require approval for dangerous terminal commands.
    pub approval: bool,
    /// Context budget (tokens) before compression kicks in.
    pub context_budget_tokens: usize,
    /// Persist messages into the SQLite store.
    pub persist: bool,
    /// Source tag for created sessions ("cli", "cron", "delegate").
    pub source: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: 90,
            max_tokens: None,
            temperature: None,
            system_prompt: None,
            strip_thinking_blocks: true,
            model: None,
            concurrent_tool_execution: false,
            max_concurrent_tools: 5,
            approval: true,
            context_budget_tokens: 120_000,
            persist: true,
            source: "cli".to_string(),
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
    /// Called when an approval prompt is raised (informational).
    pub on_approval_request: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl Default for AgentCallbacks {
    fn default() -> Self {
        Self {
            on_tool_start: None,
            on_tool_complete: None,
            on_stream_delta: None,
            on_thinking: None,
            on_step: None,
            on_approval_request: None,
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
    /// Session id (when persistence is wired)
    pub session_id: Option<String>,
}

/// Record of a tool call
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: serde_json::Value,
}

/// The main agent — orchestrates conversation flow
pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Arc<Mutex<ToolRegistry>>,
    config: AgentConfig,
    callbacks: Arc<Mutex<AgentCallbacks>>,
    context: Arc<ToolContext>,
    store: Option<Arc<SqliteSessionStore>>,
    /// Delegation depth (0 = top-level).
    depth: usize,
}

impl Agent {
    /// Create a new agent with a provider and tool registry
    pub fn new(provider: Arc<dyn Provider>, tools: ToolRegistry) -> Self {
        let context = ToolContext::new().with_provider(provider.clone());
        Self {
            provider,
            tools: Arc::new(Mutex::new(tools)),
            config: AgentConfig::default(),
            callbacks: Arc::new(Mutex::new(AgentCallbacks::default())),
            context: Arc::new(context),
            store: None,
            depth: 0,
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

    /// Set the shared tool context (session id, workdir, callbacks...).
    pub fn with_tool_context(mut self, context: ToolContext) -> Self {
        let mut context = context;
        context.provider = Some(self.provider.clone());
        self.context = Arc::new(context);
        self
    }

    /// Attach the SQLite store (enables session_search + persistence).
    pub fn with_store(mut self, store: Arc<SqliteSessionStore>) -> Self {
        let context = Arc::make_mut(&mut self.context);
        context.store = Some(store.clone());
        self.store = Some(store);
        self
    }

    pub fn tool_context(&self) -> Arc<ToolContext> {
        self.context.clone()
    }

    /// The attached SQLite store, if any.
    pub fn store(&self) -> Option<Arc<SqliteSessionStore>> {
        self.store.clone()
    }

    /// Wire this agent as its own delegation + cron runner. Call once after
    /// wrapping the agent in an Arc.
    pub fn wire_runners(self: &Arc<Self>) {
        self.context.set_subagent_runner(self.clone());
        self.context.set_cron_runner(self.clone());
    }

    /// Build the effective system prompt: configured prompt + memory
    /// injection (hermes injects MEMORY.md/USER.md into every turn).
    fn effective_system_prompt(&self) -> Option<String> {
        let base = self.config.system_prompt.clone().unwrap_or_else(|| {
            "You are ulnclaw, a capable AI assistant with tools for shell, files, web, \
             memory, skills, and scheduling. Be concise and precise."
                .to_string()
        });
        let mut parts = vec![base];
        if let Some(memory) = crate::tools::builtin::memory::load_memory_for_prompt(&self.context.home) {
            parts.push(format!("## Persistent memory\n{}", memory));
        }
        parts.push(format!(
            "## Environment\n- cwd: {}\n- home: {}",
            self.context.cwd().display(),
            self.context.home.display()
        ));
        Some(parts.join("\n\n"))
    }

    /// Run the agent with a user message
    pub async fn run(
        &self,
        user_message: &str,
        conversation_history: Option<Vec<Message>>,
    ) -> Result<RunResult> {
        self.run_with_session(user_message, conversation_history, None).await
    }

    /// Run the agent, optionally resuming an existing session id instead of
    /// creating a fresh one (used by the HTTP gateway for continuity).
    pub async fn run_with_session(
        &self,
        user_message: &str,
        conversation_history: Option<Vec<Message>>,
        resume_session_id: Option<&str>,
    ) -> Result<RunResult> {
        let mut messages = Vec::new();
        let mut total_usage = Usage::default();
        let mut tool_calls_made = Vec::new();

        // System prompt (with memory injection)
        if let Some(prompt) = self.effective_system_prompt() {
            messages.push(Message {
                role: Role::System,
                content: Some(prompt),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        if let Some(history) = conversation_history {
            messages.extend(history);
        }

        messages.push(Message {
            role: Role::User,
            content: Some(user_message.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        // Persistence: create/resume a session row.
        let resuming = resume_session_id.is_some();
        let session_id = if self.config.persist {
            if let Some(ref store) = self.store {
                let model = self
                    .config
                    .model
                    .clone()
                    .unwrap_or_else(|| self.provider.model().to_string());
                if let Some(sid) = resume_session_id {
                    if let Err(e) = store.ensure_session(
                        sid,
                        &self.config.source,
                        Some(&model),
                        Some(&self.context.cwd().display().to_string()),
                    ) {
                        warn!("session ensure failed: {}", e);
                    }
                    Some(sid.to_string())
                } else {
                    match store.create_session(&self.config.source, Some(&model), Some(&self.context.cwd().display().to_string())) {
                        Ok(id) => Some(id),
                        Err(e) => {
                            warn!("session create failed: {}", e);
                            None
                        }
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(ref sid) = session_id {
            // When resuming, history rows already exist in the store — only
            // append the new user message (system prompt + history are known).
            let start = if resuming {
                messages.len().saturating_sub(1)
            } else {
                0
            };
            for message in &messages[start..] {
                if let Some(ref store) = self.store {
                    store.append_message(sid, message).ok();
                }
            }
        }

        let compressor = crate::context::ContextCompressor::new(self.config.context_budget_tokens);

        // Main conversation loop
        for iteration in 0..self.config.max_iterations {
            debug!("Agent iteration {}", iteration);

            // Reset checkpoint per-turn dedup (hermes new_turn()).
            self.context.checkpoint_manager().new_turn();

            // Context compression when over budget.
            if compressor.needs_compression(&messages) {
                if let Some(compressed) = compressor
                    .compress_with_provider(messages.clone(), self.provider.as_ref())
                    .await
                {
                    messages = compressed;
                    debug!("Context compressed");
                }
            }

            let response = self.call_provider(&messages).await?;

            if let Some(ref usage) = response.usage {
                total_usage.merge(usage);
                if let (Some(store), Some(sid)) = (self.store.as_ref(), session_id.as_ref()) {
                    store
                        .update_usage(sid, usage.prompt_tokens, usage.completion_tokens, 0)
                        .ok();
                }
            }

            if !response.tool_calls.is_empty() {
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.content.clone(),
                    tool_calls: Some(response.tool_calls.clone()),
                    tool_call_id: None,
                    name: None,
                });
                if let (Some(store), Some(sid)) = (self.store.as_ref(), session_id.as_ref()) {
                    let msg = messages.last().unwrap();
                    store.append_message(sid, msg).ok();
                }

                let tool_results = self.execute_tool_calls(&response.tool_calls).await?;

                for (tool_call, result) in response.tool_calls.iter().zip(tool_results.iter()) {
                    let tool_message = Message {
                        role: Role::Tool,
                        content: Some(serde_json::to_string(result).unwrap_or_default()),
                        tool_calls: None,
                        tool_call_id: Some(tool_call.id.clone()),
                        name: Some(tool_call.function.name.clone()),
                    };
                    messages.push(tool_message);
                    if let (Some(store), Some(sid)) = (self.store.as_ref(), session_id.as_ref()) {
                        store.append_message(sid, messages.last().unwrap()).ok();
                    }

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

                if let (Some(store), Some(sid)) = (self.store.as_ref(), session_id.as_ref()) {
                    store
                        .update_usage(sid, 0, 0, response.tool_calls.len() as u32)
                        .ok();
                }

                let callbacks = self.callbacks.lock().await;
                if let Some(ref on_step) = callbacks.on_step {
                    on_step(iteration as u32 + 1);
                }
                continue;
            }

            let mut content = response.content.unwrap_or_default();
            if self.config.strip_thinking_blocks {
                content = strip_thinking_blocks(&content);
            }

            messages.push(Message {
                role: Role::Assistant,
                content: Some(content.clone()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
            if let (Some(store), Some(sid)) = (self.store.as_ref(), session_id.as_ref()) {
                store.append_message(sid, messages.last().unwrap()).ok();
                store.end_session(sid, "complete").ok();
            }

            return Ok(RunResult {
                content,
                conversation: messages,
                usage: total_usage,
                iterations: iteration + 1,
                tool_calls: tool_calls_made,
                session_id,
            });
        }

        if let (Some(store), Some(sid)) = (self.store.as_ref(), session_id.as_ref()) {
            store.end_session(sid, "max_iterations").ok();
        }

        Ok(RunResult {
            content: format!(
                "Reached the iteration budget ({} turns) before finishing.",
                self.config.max_iterations
            ),
            conversation: messages,
            usage: total_usage,
            iterations: self.config.max_iterations,
            tool_calls: tool_calls_made,
            session_id,
        })
    }

    /// Simple interface — just return the text response
    pub async fn chat(&self, user_message: &str) -> Result<String> {
        let result = self.run(user_message, None).await?;
        Ok(result.content)
    }

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
            stream: STREAM_EMITTER.try_with(|_| ()).is_ok(),
            stop: None,
        };

        // Non-streaming path (no active stream consumer, or provider
        // doesn't support streaming).
        if !request.stream || !self.provider.supports_streaming() {
            return self.provider.chat_completion(request).await;
        }

        // Streaming path: accumulate chunks into a ProviderResponse while
        // emitting content deltas to the stream consumer.
        use futures::TryStreamExt;
        let mut stream = self.provider.chat_completion_stream(request).await?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_deltas: Vec<crate::provider::ToolCallDelta> = Vec::new();
        let mut finish_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut model: Option<String> = None;
        while let Some(chunk) = stream.try_next().await? {
            if let Some(delta) = &chunk.delta_content {
                if !delta.is_empty() {
                    emit_stream_event(StreamEvent::Delta(delta.clone()));
                    {
                        let callbacks = self.callbacks.lock().await;
                        if let Some(ref on_delta) = callbacks.on_stream_delta {
                            on_delta(delta);
                        }
                    }
                    content.push_str(delta);
                }
            }
            if let Some(delta) = &chunk.delta_reasoning {
                reasoning.push_str(delta);
            }
            tool_deltas.extend(chunk.tool_call_deltas);
            if chunk.finish_reason.is_some() {
                finish_reason = chunk.finish_reason;
            }
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
            if chunk.model.is_some() {
                model = chunk.model;
            }
        }
        let tool_calls = crate::provider::assemble_tool_calls(&tool_deltas);
        Ok(crate::provider::ProviderResponse {
            content: if content.is_empty() { None } else { Some(content) },
            tool_calls,
            usage,
            model: model.unwrap_or_else(|| self.provider.model().to_string()),
            reasoning: if reasoning.is_empty() { None } else { Some(reasoning) },
            finish_reason,
        })
    }

    /// Approval gate for terminal commands (port of hermes approval flow).
    async fn approval_check(&self, tool_name: &str, args: &serde_json::Value) -> Option<serde_json::Value> {
        if tool_name != "terminal" || !self.config.approval {
            return None;
        }
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            return None;
        }
        match classify_command(command) {
            ApprovalDecision::Allow => None,
            ApprovalDecision::Block(reason) => Some(serde_json::json!({
                "success": false,
                "error": format!(
                    "BLOCKED: this command matches the hardline safety floor ({}). It cannot be approved. Use a safer alternative.",
                    reason
                ),
            })),
            ApprovalDecision::Confirm(reason) => {
                {
                    let callbacks = self.callbacks.lock().await;
                    if let Some(ref hook) = callbacks.on_approval_request {
                        hook(command);
                    }
                }
                let approve = self.context.approve.clone();
                let approved = match approve {
                    Some(callback) => callback(reason.to_string(), command.to_string()).await,
                    None => false,
                };
                if approved {
                    None
                } else {
                    Some(serde_json::json!({
                        "success": false,
                        "error": format!(
                            "Command rejected by approval policy ({}). The user declined to run it. Do not retry the same command.",
                            reason
                        ),
                    }))
                }
            }
        }
    }

    async fn execute_tool_calls(
        &self,
        tool_calls: &[crate::provider::ToolCall],
    ) -> Result<Vec<serde_json::Value>> {
        let mut results = Vec::new();

        for tool_call in tool_calls {
            let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
                .unwrap_or(serde_json::json!({}));

            {
                let callbacks = self.callbacks.lock().await;
                if let Some(ref on_tool_start) = callbacks.on_tool_start {
                    on_tool_start(&tool_call.function.name, &args);
                }
            }

            // Transparent checkpoint before file-mutating tools (hermes
            // checkpoint_manager hook): snapshot the project once per turn
            // before the first write_file/patch.
            if matches!(tool_call.function.name.as_str(), "write_file" | "patch") {
                let manager = self.context.checkpoint_manager();
                if manager.enabled() {
                    let target = args
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|raw| self.context.resolve_path(raw))
                        .unwrap_or_else(|| self.context.cwd());
                    let workdir = manager.working_dir_for_path(&target);
                    manager
                        .ensure_checkpoint(
                            &workdir.to_string_lossy(),
                            &format!("before {} ({})", tool_call.function.name, target.display()),
                        )
                        .await;
                }
            }

            emit_stream_event(StreamEvent::ToolProgress {
                tool: tool_call.function.name.clone(),
                status: "started".to_string(),
            });
            emit_stream_event(StreamEvent::ToolStarted {
                name: tool_call.function.name.clone(),
                call_id: tool_call.id.clone(),
                arguments: tool_call.function.arguments.clone(),
            });

            // Approval gate before dispatch.
            let result_value = if let Some(blocked) = self
                .approval_check(&tool_call.function.name, &args)
                .await
            {
                blocked
            } else {
                let tools = self.tools.lock().await;
                match tools
                    .dispatch(&tool_call.function.name, args.clone(), self.context.clone())
                    .await
                {
                    Ok(value) => value,
                    Err(e) => {
                        warn!("Tool {} failed: {}", tool_call.function.name, e);
                        serde_json::json!({"error": e.to_string(), "success": false})
                    }
                }
            };

            {
                let callbacks = self.callbacks.lock().await;
                if let Some(ref on_tool_complete) = callbacks.on_tool_complete {
                    on_tool_complete(&tool_call.function.name, &result_value);
                }
            }
            emit_stream_event(StreamEvent::ToolProgress {
                tool: tool_call.function.name.clone(),
                status: "completed".to_string(),
            });
            emit_stream_event(StreamEvent::ToolCompleted {
                call_id: tool_call.id.clone(),
                result: serde_json::to_string(&result_value).unwrap_or_default(),
            });

            results.push(result_value);
        }

        Ok(results)
    }
}

#[async_trait]
impl SubAgentRunner for Agent {
    async fn run_subagent(&self, goal: &str, context: &str) -> Result<String> {
        let delegation = &self.context.config.delegation;
        if self.depth >= delegation.max_depth {
            return Err(AgentError::Tool(format!(
                "Delegation depth limit reached ({}); cannot spawn further sub-agents.",
                delegation.max_depth
            )));
        }

        // The child shares this agent's tool registry (Arc clone below).
        let child_session = if let Some(ref store) = self.store {
            store
                .create_child_session(
                    &self.context.session_id,
                    "delegate",
                    Some(self.provider.model()),
                )
                .ok()
        } else {
            None
        };

        let mut child_context = ToolContext::new()
            .with_session_id(child_session.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()))
            .with_home(self.context.home.clone())
            .with_config(self.context.config.clone())
            .with_provider(self.provider.clone());
        child_context.workdir = self.context.workdir.clone();
        if let Some(ref store) = self.store {
            child_context = child_context.with_store(store.clone());
        }
        if let Some(ref approve) = self.context.approve {
            child_context = child_context.with_approve(approve.clone());
        }
        // Sub-agents get no clarify (no user present) and no nested runners.
        let child = Agent {
            provider: self.provider.clone(),
            tools: self.tools.clone(),
            config: AgentConfig {
                max_iterations: delegation.child_max_iterations,
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
                system_prompt: Some(
                    "You are a focused sub-agent. Complete the delegated goal completely and \
                     report a single final answer. You cannot ask the user questions."
                        .to_string(),
                ),
                strip_thinking_blocks: true,
                model: self.config.model.clone(),
                concurrent_tool_execution: self.config.concurrent_tool_execution,
                max_concurrent_tools: self.config.max_concurrent_tools,
                approval: self.config.approval,
                context_budget_tokens: self.config.context_budget_tokens,
                persist: false,
                source: "delegate".to_string(),
            },
            callbacks: Arc::new(Mutex::new(AgentCallbacks::default())),
            context: Arc::new(child_context),
            store: None,
            depth: self.depth + 1,
        };

        let prompt = if context.is_empty() {
            goal.to_string()
        } else {
            format!("{}\n\nContext:\n{}", goal, context)
        };
        let result = child.run(&prompt, None).await?;
        if let (Some(store), Some(sid)) = (self.store.as_ref(), child_session.as_ref()) {
            store
                .append_message(
                    &sid,
                    &Message {
                        role: Role::Assistant,
                        content: Some(result.content.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    },
                )
                .ok();
        }
        Ok(result.content)
    }
}

#[async_trait]
impl CronRunner for Agent {
    async fn run_prompt(&self, prompt: &str, skills: &[String]) -> Result<String> {
        // Load requested skills into the prompt preamble.
        let mut preamble = String::new();
        let skills_dir = self.context.home.join("skills");
        for name in skills {
            if let Some(skill) = crate::skills::find_skill(&skills_dir, name) {
                let content = std::fs::read_to_string(skill.path.join("SKILL.md")).unwrap_or_default();
                preamble.push_str(&format!("## Skill: {}\n{}\n\n", skill.name, content));
            }
        }
        let full_prompt = if preamble.is_empty() {
            prompt.to_string()
        } else {
            format!("{}\n\n# Task\n{}", preamble, prompt)
        };

        let cron_agent = Agent {
            provider: self.provider.clone(),
            tools: self.tools.clone(),
            config: AgentConfig {
                max_iterations: self.config.max_iterations,
                system_prompt: Some(
                    "You are running as an autonomous cron job. No user is present: never ask \
                     questions, complete the task with your best judgment, and put the primary \
                     user-facing content in your final response. Do NOT schedule more cron jobs."
                        .to_string(),
                ),
                approval: false,
                persist: true,
                source: "cron".to_string(),
                ..self.config.clone()
            },
            callbacks: Arc::new(Mutex::new(AgentCallbacks::default())),
            context: self.context.clone(),
            store: self.store.clone(),
            depth: self.depth,
        };
        let result = cron_agent.run(&full_prompt, None).await?;
        Ok(result.content)
    }
}

/// Strip thinking/reasoning blocks from text
pub fn strip_thinking_blocks(text: &str) -> String {
    let mut result = text.to_string();
    for (open, close) in [("<thinking>", "</thinking>"), ("<think>", "</think>")] {
        while let Some(start) = result.find(open) {
            if let Some(end) = result[start..].find(close) {
                result = format!(
                    "{}{}",
                    &result[..start],
                    &result[start + end + close.len()..]
                );
            } else {
                result = result[..start].to_string();
                break;
            }
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_thinking() {
        assert_eq!(
            strip_thinking_blocks("hello <thinking>hm</thinking> world"),
            "hello  world"
        );
        assert_eq!(strip_thinking_blocks("<think>x</think>done"), "done");
    }
}
