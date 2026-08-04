//! ulnclaw CLI — port of the hermes CLI core (chat REPL, one-shot runs,
//! session/skill/cron/tool management).

use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use ulnclaw::agent::{Agent, AgentConfig};
use ulnclaw::config::UlncLawConfig;
use ulnclaw::provider::openai::OpenAiProvider;
use ulnclaw::provider::{Message, Role};
use ulnclaw::session::sqlite::SqliteSessionStore;
use ulnclaw::tools::builtin::register_builtin_tools;
use ulnclaw::tools::context::ToolContext;
use ulnclaw::tools::ToolRegistry;
use ulnclaw::session::SessionStore;
use ulnclaw::toolsets;

#[derive(Parser)]
#[command(name = "ulnclaw", version, about = "ulnclaw — Rust AI agent engine (hermes-agent port)")]
struct Cli {
    /// Config file path (default: ~/.ulnclaw/config.toml)
    #[arg(long, global = true)]
    config: Option<String>,
    /// Named profile from config.toml [profiles]
    #[arg(long, global = true)]
    profile: Option<String>,
    /// Verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive chat REPL (default)
    Chat,
    /// One-shot run: ulnclaw run "your prompt"
    Run { prompt: Vec<String> },
    /// Session management
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// List registered tools and toolsets
    Tools,
    /// Skill management
    Skills {
        #[command(subcommand)]
        action: Option<SkillAction>,
    },
    /// Cron job management
    Cron {
        #[command(subcommand)]
        action: Option<CronAction>,
    },
    /// Start the HTTP gateway (OpenAI-compatible API server)
    Gateway {
        /// Bind host (overrides [gateway] host / ULNCLAW_GATEWAY_HOST)
        #[arg(long)]
        host: Option<String>,
        /// Bind port (overrides [gateway] port / ULNCLAW_GATEWAY_PORT)
        #[arg(long)]
        port: Option<u16>,
    },
    /// Filesystem checkpoint management (snapshot list/restore/prune)
    Checkpoints {
        #[command(subcommand)]
        action: CheckpointAction,
    },
    /// Git working-tree diff (hermes working_diff): what changed here?
    Diff {
        /// Show staged changes only (`git diff --cached`)
        #[arg(long)]
        staged: bool,
        /// Show everything since HEAD (staged + unstaged + untracked)
        #[arg(long)]
        all: bool,
        /// Directory to inspect (default: cwd)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Optional pathspecs to restrict the diff
        paths: Vec<String>,
    },
    /// Mixture-of-Agents: fan out reference models + aggregator synthesis
    /// (hermes `moa` presets / `/moa` one-shot)
    Moa {
        #[command(subcommand)]
        action: Option<MoaAction>,
    },
    /// Write a default config.toml
    Init,
}

#[derive(Subcommand)]
enum SessionAction {
    /// List recent sessions
    List { #[arg(long, default_value = "20")] limit: usize },
    /// Show a session's messages
    Show { id: String },
    /// Full-text search across sessions
    Search { query: Vec<String> },
    /// Export a session as verifiable Markdown (hermes session export)
    Export {
        id: String,
        /// Output directory (default: <home>/exports)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Export format: md or html
        #[arg(long, default_value = "md")]
        format: String,
        /// Skip the SHA256 verification footer
        #[arg(long)]
        no_verification: bool,
    },
    /// Recap recent activity in a session (local, no LLM call)
    Recap { id: String },
    /// Offline non-destructive recovery of a damaged state.db
    /// (hermes session_recovery)
    Recover {
        /// Damaged database file (copied first; never opened in place)
        source: PathBuf,
        /// Output path (must not exist; default: <source>.recovered.db)
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum MoaAction {
    /// Run one prompt through a MoA preset and print the synthesis
    Run {
        prompt: Vec<String>,
        /// Preset name (default: `moa.default_preset` or "default")
        #[arg(long)]
        preset: Option<String>,
    },
    /// Show configured presets
    List,
    /// Delete a preset from config.toml
    Delete { name: String },
}

#[derive(Subcommand)]
enum SkillAction {
    List,
    View { name: String },
    /// List installed skills that declare a blueprint schedule
    Blueprints,
    /// Schedule a blueprint skill as a cron job (hermes blueprint jobs)
    Schedule {
        name: String,
        /// Custom job name (default: `blueprint:<skill>`)
        #[arg(long)]
        job_name: Option<String>,
    },
    /// Remove the cron job created for a blueprint skill
    Unschedule { name: String },
}

#[derive(Subcommand)]
enum CronAction {
    List,
    Remove { id: String },
    Pause { id: String },
    Resume { id: String },
}

#[derive(Subcommand)]
enum CheckpointAction {
    /// List checkpoints for a directory (default: cwd)
    List {
        /// Directory to inspect (default: current directory)
        dir: Option<String>,
    },
    /// Show the shared checkpoint store status (projects, sizes)
    Status,
    /// Restore a directory (or a single file) to a checkpoint
    Restore {
        /// Checkpoint hash (short or full)
        hash: String,
        /// Optional single file to restore
        file: Option<String>,
        /// Directory the checkpoint belongs to (default: cwd)
        #[arg(long)]
        dir: Option<String>,
    },
    /// Preview the diff between the working tree and a checkpoint
    Diff {
        /// Checkpoint hash (short or full)
        hash: String,
        /// Directory the checkpoint belongs to (default: cwd)
        #[arg(long)]
        dir: Option<String>,
    },
    /// Delete orphan/stale checkpoints and reclaim store space
    Prune {
        /// Retention window in days (default: [checkpoints] retention_days)
        #[arg(long)]
        days: Option<u64>,
    },
}

fn load_config(cli: &Cli) -> UlncLawConfig {
    let mut config = UlncLawConfig::load(cli.config.as_ref().map(std::path::Path::new))
        .unwrap_or_default();
    if let Some(ref profile) = cli.profile {
        config = config.with_profile(profile);
    }
    config
}

fn build_provider(config: &UlncLawConfig) -> Result<Arc<dyn ulnclaw::provider::Provider>, String> {
    let api_key = config.resolve_api_key();
    let keyless = matches!(
        config.model.provider.as_str(),
        "ollama" | "llamacpp" | "llama_cpp" | "local"
    );
    if api_key.is_none() && !keyless {
        return Err(
            "No API key found. Set OPENAI_API_KEY / ANTHROPIC_API_KEY (or api_key in config.toml)."
                .into(),
        );
    }
    if config.model.provider == "anthropic" {
        let mut builder = ulnclaw::provider::anthropic::AnthropicProvider::builder()
            .endpoint(&config.resolve_base_url())
            .model(&config.model.model)
            .name(&config.model.provider)
            .max_retries(config.model.max_retries);
        if let Some(ref key) = api_key {
            builder = builder.api_key(key);
        }
        return Ok(Arc::new(builder.build().map_err(|e| e.to_string())?));
    }
    let mut builder = OpenAiProvider::builder()
        .endpoint(&config.resolve_base_url())
        .model(&config.model.model)
        .name(&config.model.provider)
        .max_retries(config.model.max_retries);
    if let Some(ref key) = api_key {
        builder = builder.api_key(key);
    }
    let provider = builder.build().map_err(|e| e.to_string())?;
    Ok(Arc::new(provider))
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    let config = load_config(&cli);
    let result = dispatch(cli, config).await;
    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

async fn gateway_cmd(
    config: &UlncLawConfig,
    host: Option<String>,
    port: Option<u16>,
) -> Result<(), String> {
    let mut gateway = config.gateway.resolved();
    if let Some(host) = host {
        gateway.host = host;
    }
    if let Some(port) = port {
        gateway.port = port;
    }
    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    let router = ulnclaw::gateway::ApprovalRouter::with_options(
        std::time::Duration::from_secs(config.approvals.timeout),
        Some(home.join("approvals.json")),
    );
    let state_holder: Arc<tokio::sync::OnceCell<Arc<ulnclaw::gateway::GatewayState>>> =
        Arc::new(tokio::sync::OnceCell::new());
    let approve = ulnclaw::gateway::gateway_approve_fn(router.clone(), state_holder.clone());
    let agent = make_agent(config, false, Some(approve)).await?;
    let state = ulnclaw::gateway::GatewayState::new(
        agent,
        config.model.model.clone(),
        config.model.provider.clone(),
        gateway.key.clone(),
        router,
    )
    .map_err(|e| e.to_string())?;
    state_holder.set(state.clone()).ok();
    let cron_store =
        ulnclaw::cron::CronStore::open(&home.join("state.db")).map_err(|e| e.to_string())?;
    state.cron.set(std::sync::Arc::new(cron_store)).ok();
    state.skills_dir.set(home.join("skills")).ok();
    ulnclaw::gateway::serve(state, &gateway.host, gateway.port)
        .await
        .map_err(|e| e.to_string())
}

async fn checkpoints_cmd(config: &UlncLawConfig, action: CheckpointAction) -> Result<(), String> {
    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    let manager =
        ulnclaw::checkpoint::CheckpointManager::new(home.join("checkpoints"), &config.checkpoints);
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    match action {
        CheckpointAction::List { dir } => {
            let dir = dir.unwrap_or(cwd);
            let checkpoints = manager.list_checkpoints(&dir).await;
            println!("{}", ulnclaw::checkpoint::format_checkpoint_list(&checkpoints, &dir));
        }
        CheckpointAction::Status => {
            let status = manager.status().await;
            println!(
                "{}",
                serde_json::to_string_pretty(&status).map_err(|e| e.to_string())?
            );
        }
        CheckpointAction::Restore { hash, file, dir } => {
            let dir = dir.unwrap_or(cwd);
            let result = manager
                .restore(&dir, &hash, file.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            println!(
                "✅ Restored {} to {} ({})",
                dir,
                result["restored_to"].as_str().unwrap_or("?"),
                result["reason"].as_str().unwrap_or("?")
            );
        }
        CheckpointAction::Diff { hash, dir } => {
            let dir = dir.unwrap_or(cwd);
            let result = manager.diff(&dir, &hash).await.map_err(|e| e.to_string())?;
            let stat = result["stat"].as_str().unwrap_or("");
            if !stat.is_empty() {
                println!("{}", stat);
            }
            println!("{}", result["diff"].as_str().unwrap_or(""));
        }
        CheckpointAction::Prune { days } => {
            let days = days.unwrap_or(config.checkpoints.retention_days);
            let stats = manager.prune(days, true).await;
            println!(
                "scanned: {}, deleted orphan: {}, deleted stale: {}, freed: {} bytes",
                stats.scanned, stats.deleted_orphan, stats.deleted_stale, stats.bytes_freed
            );
        }
    }
    Ok(())
}

fn init_logging(verbose: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

async fn dispatch(cli: Cli, config: UlncLawConfig) -> Result<(), String> {
    match cli.command.unwrap_or(Commands::Chat) {
        Commands::Chat => chat_repl(&config).await,
        Commands::Run { prompt } => {
            let prompt = prompt.join(" ");
            if prompt.is_empty() {
                return Err("usage: ulnclaw run \"your prompt\"".into());
            }
            one_shot(&config, &prompt).await
        }
        Commands::Sessions { action } => sessions_cmd(action).await,
        Commands::Tools => tools_cmd(&config),
        Commands::Skills { action } => skills_cmd(action.unwrap_or(SkillAction::List)).await,
        Commands::Cron { action } => cron_cmd(action.unwrap_or(CronAction::List)).await,
        Commands::Gateway { host, port } => gateway_cmd(&config, host, port).await,
        Commands::Checkpoints { action } => checkpoints_cmd(&config, action).await,
        Commands::Diff { staged, all, dir, paths } => {
            let mode = if all {
                ulnclaw::git_diff::DiffMode::All
            } else if staged {
                ulnclaw::git_diff::DiffMode::Staged
            } else {
                ulnclaw::git_diff::DiffMode::Working
            };
            let cwd = dir.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            match ulnclaw::git_diff::collect_working_diff(&cwd, mode, &paths) {
                Ok(result) if result.empty => {
                    println!("No changes ({} mode).", mode.as_str());
                }
                Ok(result) => {
                    if !result.stat.is_empty() {
                        println!("{}", result.stat);
                        println!();
                    }
                    if !result.diff.is_empty() {
                        println!("{}", result.diff);
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
            Ok(())
        }
        Commands::Moa { action } => {
            moa_cmd(&config, action.unwrap_or(MoaAction::List), cli.config.as_deref()).await
        }
        Commands::Init => {
            let path = UlncLawConfig::write_default_if_missing().map_err(|e| e.to_string())?;
            println!("config written to {}", path.display());
            Ok(())
        }
    }
}

async fn make_agent(
    config: &UlncLawConfig,
    interactive: bool,
    approve_override: Option<ulnclaw::tools::context::ApproveFn>,
) -> Result<Arc<Agent>, String> {
    let provider = build_provider(config)?;
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);

    // MCP servers: connect and register their tools (hermes mcp_tool.py).
    for server in &config.mcp.servers {
        match ulnclaw::mcp::register_mcp_server(&mut registry, server).await {
            Ok(count) => eprintln!("[mcp] {}: {} tools registered", server.name, count),
            Err(e) => eprintln!("[mcp] {}: unavailable ({})", server.name, e),
        }
    }

    toolsets::apply_toolset_policy(&mut registry, &config.enabled_toolsets, &config.disabled_toolsets);

    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    let store = Arc::new(SqliteSessionStore::open(home.join("state.db")).map_err(|e| e.to_string())?);

    let mut context = ToolContext::new()
        .with_home(home)
        .with_config(config.clone())
        .with_env_passthrough(&config.terminal.env_passthrough)
        .with_store(store.clone())
        .with_provider(provider.clone());

    if interactive {
        context = context.with_clarify(Arc::new(|question, choices, multi| {
            Box::pin(async move {
                println!("\n❓ {}", question);
                if !choices.is_empty() {
                    for (i, choice) in choices.iter().enumerate() {
                        println!("  {}. {}", i + 1, choice);
                    }
                }
                if multi {
                    println!("(multiple allowed)");
                }
                print!("> ");
                std::io::stdout().flush().ok();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).ok();
                let answer = line.trim().to_string();
                if let Ok(idx) = answer.parse::<usize>() {
                    if idx >= 1 && idx <= choices.len() {
                        return Ok(choices[idx - 1].clone());
                    }
                }
                Ok(answer)
            })
        }));
        if context.approve.is_none() {
            context = context.with_approve(Arc::new(|reason, command| {
            Box::pin(async move {
                println!("\n⚠️  Approve dangerous command? [{}]\n{}\nApprove? [y/N]", reason, command);
                print!("> ");
                std::io::stdout().flush().ok();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).ok();
                matches!(line.trim(), "y" | "Y" | "yes" | "YES")
            })
        }));
        }
    }
    if let Some(approve) = approve_override {
        context = context.with_approve(approve);
    }

    // Checkpoint auto-maintenance in the background (hermes startup hook).
    if config.checkpoints.enabled {
        let manager = context.checkpoint_manager();
        tokio::spawn(async move {
            manager.maybe_auto_prune().await;
        });
    }

    // Capture the tool catalog snapshot for tool_search (before registry moves).
    context.set_tool_definitions(registry.definitions());
    let agent = Agent::new(provider.clone(), registry).with_config(AgentConfig {
        max_iterations: config.agent.max_iterations,
        system_prompt: None,
        concurrent_tool_execution: config.agent.concurrent_tool_execution,
        max_concurrent_tools: config.agent.max_concurrent_tools,
        approval: config.agent.approval,
        context_budget_tokens: config.agent.context_budget_tokens,
        persist: true,
        source: "cli".to_string(),
        environment_probe: config.agent.environment_probe,
        terminal_backend: config.terminal.backend.clone().unwrap_or_else(|| "local".to_string()),
        ..Default::default()
    });
    let agent = agent
        .with_store(store)
        .with_tool_context(context)
        .with_fallback_specs(&config.model.fallbacks);
    let agent = Arc::new(agent);
    // Wire the runners now that the agent is in an Arc.
    agent.wire_runners();
    Ok(agent)
}

async fn one_shot(config: &UlncLawConfig, prompt: &str) -> Result<(), String> {
    let agent = make_agent(config, false, None).await?;
    let result = agent.run(prompt, None).await.map_err(|e| e.to_string())?;
    println!("{}", result.content);
    Ok(())
}

async fn chat_repl(config: &UlncLawConfig) -> Result<(), String> {
    let agent = make_agent(config, true, None).await?;
    println!(
        "ulnclaw {} — model: {} ({})",
        ulnclaw::VERSION,
        config.model.model,
        config.model.provider
    );
    println!("Type /help for commands, /quit to exit.");

    let mut history: Vec<Message> = Vec::new();
    let stdin = std::io::stdin();
    loop {
        print!("\n> ");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        stdin.read_line(&mut line).map_err(|e| e.to_string())?;
        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        if input.starts_with('/') {
            match handle_slash(&input, &agent, &mut history).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => {
                    println!("error: {}", e);
                    continue;
                }
            }
        }

        match agent.run(&input, Some(history.clone())).await {
            Ok(result) => {
                println!("\n{}", result.content);
                // Keep the conversation going (drop system prompt from history).
                history = result
                    .conversation
                    .into_iter()
                    .filter(|m| m.role != Role::System)
                    .collect();
            }
            Err(e) => println!("error: {}", e),
        }
    }
    Ok(())
}

async fn handle_slash(input: &str, agent: &Arc<Agent>, history: &mut Vec<Message>) -> Result<bool, String> {
    let mut parts = input.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match cmd {
        "/quit" | "/exit" | "/q" => return Ok(false),
        "/new" => {
            history.clear();
            println!("New conversation started.");
        }
        "/help" => {
            println!(
                "Commands:\n  /new            start a fresh conversation\n  /history        show turn count\n  /recap          recap recent activity in this conversation\n  /moa <prompt>   one-shot Mixture-of-Agents synthesis (default preset)\n  /search <text>  search past sessions\n  /tools          list enabled tools\n  /skills         list skills\n  /memory         show persistent memory\n  /sessions       list recent sessions\n  /usage          token usage of this conversation\n  /rollback [N|hash] [file]   list/restore checkpoints (hermes-style)\n  /rollback diff <N|hash>     preview changes since a checkpoint\n  /diff [N|hash|session]      cumulative session diff / vs a checkpoint\n  /gitdiff [staged|all]     git working-tree diff (what changed here?)\n  /quit           exit"
            );
        }
        "/history" => {
            println!("{} messages in current conversation.", history.len());
        }
        "/recap" => {
            println!("{}", ulnclaw::session::recap::build_recap(history, None, None));
        }
        "/moa" => {
            if rest.is_empty() {
                println!("usage: /moa <prompt>  (runs one prompt through the default MoA preset)");
            } else {
                let config = agent.tool_context().config.clone();
                match ulnclaw::moa::run_moa(&config, rest, None).await {
                    Ok(outcome) => {
                        for reference in &outcome.references {
                            if reference.failed() {
                                eprintln!("  ✗ {} failed", reference.label);
                            } else {
                                eprintln!("  ✓ {}", reference.label);
                            }
                        }
                        println!("{}", outcome.synthesis);
                    }
                    Err(e) => println!("moa failed: {}", e),
                }
            }
        }
        "/usage" => {
            println!("(usage is tracked per session in state.db; see `ulnclaw sessions list`)");
        }
        "/search" => {
            if rest.is_empty() {
                println!("usage: /search <text>");
            } else if let Some(store) = agent.tool_context().store.clone() {
                match store.search_messages(rest, 10) {
                    Ok(hits) => {
                        if hits.is_empty() {
                            println!("No matches.");
                        }
                        for (session_id, snippet) in hits {
                            println!("[{}] {}", &session_id[..session_id.len().min(8)], snippet);
                        }
                    }
                    Err(e) => println!("search failed: {}", e),
                }
            }
        }
        "/tools" => {
            println!("(use `ulnclaw tools` outside the REPL)");
        }
        "/gitdiff" => {
            let (mode, rest) = match rest.split_whitespace().next() {
                Some("staged") => (ulnclaw::git_diff::DiffMode::Staged, ""),
                Some("all") => (ulnclaw::git_diff::DiffMode::All, ""),
                _ => (ulnclaw::git_diff::DiffMode::Working, rest),
            };
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match ulnclaw::git_diff::collect_working_diff(&cwd, mode, &[]) {
                Ok(result) if result.empty => println!("No changes ({} mode).", mode.as_str()),
                Ok(result) => {
                    if !result.stat.is_empty() {
                        println!("{}", result.stat);
                        println!();
                    }
                    println!("{}", result.diff);
                    let _ = rest;
                }
                Err(e) => println!("gitdiff failed: {}", e),
            }
        }
        "/skills" => {
            let dir = agent.tool_context().home.join("skills");
            for skill in ulnclaw::skills::list_skills(&dir) {
                println!("  {} — {}", skill.name, skill.description);
            }
        }
        "/memory" => {
            match ulnclaw::tools::builtin::memory::load_memory_for_prompt(&agent.tool_context().home) {
                Some(memory) => println!("{}", memory),
                None => println!("(memory is empty)"),
            }
        }
        "/sessions" => {
            if let Some(store) = agent.tool_context().store.clone() {
                match store.list_sessions(10) {
                    Ok(sessions) => {
                        for session in sessions {
                            println!(
                                "[{}] {} messages, model={:?}",
                                &session.id[..session.id.len().min(8)],
                                session.messages.len(),
                                session.metadata.model
                            );
                        }
                    }
                    Err(e) => println!("list failed: {}", e),
                }
            }
        }
        "/rollback" => {
            let context = agent.tool_context();
            let manager = context.checkpoint_manager();
            let dir_str = context.cwd().to_string_lossy().to_string();
            let mut sub = rest.splitn(2, ' ');
            let first = sub.next().unwrap_or("").trim();
            let second = sub.next().unwrap_or("").trim();
            let checkpoints = manager.list_checkpoints(&dir_str).await;
            if first.is_empty() {
                println!(
                    "{}",
                    ulnclaw::checkpoint::format_checkpoint_list(&checkpoints, &dir_str)
                );
            } else if first == "diff" {
                match resolve_checkpoint(&checkpoints, second) {
                    Some(hash) => match manager.diff(&dir_str, &hash).await {
                        Ok(result) => print_diff_result(&result),
                        Err(e) => println!("diff failed: {}", e),
                    },
                    None => println!("no such checkpoint: {}", second),
                }
            } else {
                match resolve_checkpoint(&checkpoints, first) {
                    Some(hash) => {
                        let file = if second.is_empty() { None } else { Some(second) };
                        match manager.restore(&dir_str, &hash, file).await {
                            Ok(result) => println!(
                                "✅ Restored to {} ({})",
                                result["restored_to"].as_str().unwrap_or("?"),
                                result["reason"].as_str().unwrap_or("?")
                            ),
                            Err(e) => println!("restore failed: {}", e),
                        }
                    }
                    None => println!("no such checkpoint: {}", first),
                }
            }
        }
        "/diff" => {
            let context = agent.tool_context();
            let manager = context.checkpoint_manager();
            let dir_str = context.cwd().to_string_lossy().to_string();
            if rest.is_empty() || rest == "session" {
                print_diff_result(&manager.session_diff(&dir_str).await);
            } else {
                let checkpoints = manager.list_checkpoints(&dir_str).await;
                match resolve_checkpoint(&checkpoints, rest) {
                    Some(hash) => match manager.diff(&dir_str, &hash).await {
                        Ok(result) => print_diff_result(&result),
                        Err(e) => println!("diff failed: {}", e),
                    },
                    None => println!("no such checkpoint: {}", rest),
                }
            }
        }
        other => println!("unknown command: {} (/help for a list)", other),
    }
    Ok(true)
}

/// Resolve a checkpoint selector: 1-based list index or (prefix of a) hash.
fn resolve_checkpoint(
    checkpoints: &[ulnclaw::checkpoint::CheckpointEntry],
    selector: &str,
) -> Option<String> {
    if selector.is_empty() {
        return None;
    }
    if let Ok(n) = selector.parse::<usize>() {
        return checkpoints
            .get(n.saturating_sub(1))
            .map(|c| c.hash.clone());
    }
    checkpoints
        .iter()
        .find(|c| c.hash.starts_with(selector) || c.short_hash == selector)
        .map(|c| c.hash.clone())
}

fn print_diff_result(result: &serde_json::Value) {
    if result.get("empty").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("No changes recorded.");
        return;
    }
    let stat = result["stat"].as_str().unwrap_or("");
    let diff = result["diff"].as_str().unwrap_or("");
    if stat.is_empty() && diff.is_empty() {
        println!("No changes.");
    } else {
        if !stat.is_empty() {
            println!("{}", stat);
        }
        if !diff.is_empty() {
            println!("{}", diff);
        }
    }
}

async fn sessions_cmd(action: SessionAction) -> Result<(), String> {
    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    let store = SqliteSessionStore::open(home.join("state.db")).map_err(|e| e.to_string())?;
    match action {
        SessionAction::List { limit } => {
            let sessions = store.list_sessions(limit).map_err(|e| e.to_string())?;
            for session in sessions {
                println!(
                    "{}  msgs={:4}  model={:?}",
                    session.id,
                    session.messages.len(),
                    session.metadata.model
                );
            }
        }
        SessionAction::Show { id } => {
            let Some(session) = store.load_session(&id).map_err(|e| e.to_string())? else {
                return Err(format!("session '{}' not found", id));
            };
            for message in &session.messages {
                let content = message.content.as_deref().unwrap_or("");
                println!("── {} ──\n{}", message.role, content);
            }
        }
        SessionAction::Search { query } => {
            let query = query.join(" ");
            let hits = store.search_messages(&query, 20).map_err(|e| e.to_string())?;
            for (session_id, snippet) in hits {
                println!("[{}] {}", session_id, snippet);
            }
        }
        SessionAction::Export { id, out, format, no_verification } => {
            let row = store
                .get_session_row(&id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("session '{}' not found", id))?;
            let messages = store
                .load_messages_with_timestamps(&id)
                .map_err(|e| e.to_string())?;
            let session = ulnclaw::session::export::ExportSession {
                id: row.id.clone(),
                title: row.title.clone(),
                source: row.source.clone(),
                model: row.model.clone(),
                cwd: row.cwd.clone(),
                started_at: row.started_at,
                ended_at: row.ended_at,
                messages,
            };
            if no_verification {
                let body = match format.as_str() {
                    "html" => ulnclaw::session::export::render_session_html(&session, false),
                    _ => ulnclaw::session::export::render_session_markdown(&session, false),
                };
                println!("{}", body);
                return Ok(());
            }
            let dir = out.unwrap_or_else(|| home.join("exports"));
            let path = ulnclaw::session::export::write_session_export(&dir, &session, &format)
                .map_err(|e| e.to_string())?;
            println!("✅ Exported {} messages to {}", session.messages.len(), path.display());
        }
        SessionAction::Recover { source, out } => {
            let output = out.unwrap_or_else(|| {
                let stem = source
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("state");
                source.with_file_name(format!("{}.recovered.db", stem))
            });
            match ulnclaw::session::recovery::recover_session_database(&source, &output) {
                Ok(report) => {
                    println!("Recovery complete:");
                    println!("  source:      {}", report.source);
                    println!("  output:      {}", report.output);
                    println!("  sessions:    {}", report.sessions);
                    println!("  messages:    {}", report.messages);
                    println!(
                        "  rebuilt session rows for orphaned messages: {}",
                        report.reconstructed_sessions
                    );
                    for (table, stats) in &report.tables {
                        let mode = if stats.salvaged { "salvaged" } else { "copied" };
                        println!(
                            "  table {:<20} {} rows {} ({} skipped)",
                            table, mode, stats.copied, stats.skipped
                        );
                    }
                    println!("  integrity:   {}", if report.integrity_ok { "ok" } else { "FAILED" });
                    println!("  fts rebuilt: {}", report.fts_rebuilt);
                    match ulnclaw::session::recovery::write_recovery_report(&report) {
                        Ok(path) => println!("  report:      {}", path.display()),
                        Err(e) => eprintln!("  (report write failed: {})", e),
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        SessionAction::Recap { id } => {
            let row = store
                .get_session_row(&id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("session '{}' not found", id))?;
            let messages = store.load_messages(&id).map_err(|e| e.to_string())?;
            println!(
                "{}",
                ulnclaw::session::recap::build_recap(
                    &messages,
                    row.title.as_deref(),
                    Some(&row.id)
                )
            );
        }
    }
    Ok(())
}

fn print_moa_presets(config: &UlncLawConfig) {
    let moa = &config.moa;
    println!("Mixture of Agents presets");
    let default_name = moa
        .default_preset
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "default".to_string());
    println!("Default: {}", default_name);
    let mut names: Vec<&String> = moa.presets.keys().collect();
    names.sort();
    if names.is_empty() {
        println!("(none configured — add [moa.presets.<name>] to config.toml)");
    }
    for name in names {
        let preset = &moa.presets[name];
        let marker = if *name == default_name { "*" } else { " " };
        println!("\n{} {}", marker, name);
        println!("  Reference models:");
        for (idx, slot) in preset.reference_models.iter().enumerate() {
            let state = if slot.enabled { "" } else { " [disabled]" };
            println!("    {}. {}{}", idx + 1, slot.label(), state);
        }
        println!("  Aggregator: {}", preset.aggregator.label());
    }
}

async fn moa_cmd(
    config: &UlncLawConfig,
    action: MoaAction,
    config_path: Option<&str>,
) -> Result<(), String> {
    match action {
        MoaAction::List => {
            print_moa_presets(config);
        }
        MoaAction::Run { prompt, preset } => {
            let prompt = prompt.join(" ");
            if prompt.trim().is_empty() {
                return Err("usage: ulnclaw moa run <prompt> [--preset <name>]".into());
            }
            let outcome = ulnclaw::moa::run_moa(config, &prompt, preset.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            for reference in &outcome.references {
                if reference.failed() {
                    eprintln!("  ✗ {} failed", reference.label);
                } else {
                    eprintln!("  ✓ {}", reference.label);
                }
            }
            eprintln!("  ⇢ aggregator: {}", outcome.aggregator_label);
            println!("{}", outcome.synthesis);
        }
        MoaAction::Delete { name } => {
            let mut updated = config.clone();
            if updated.moa.presets.remove(&name).is_none() {
                return Err(format!("Unknown MoA preset: {}", name));
            }
            if updated.moa.presets.is_empty() {
                return Err("Cannot delete the only MoA preset".into());
            }
            let is_default = updated
                .moa
                .default_preset
                .as_deref()
                .map(|d| d == name)
                .unwrap_or(name == "default");
            if is_default {
                let next = updated.moa.presets.keys().next().cloned();
                updated.moa.default_preset = next;
            }
            let path = config_path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| ulnclaw::config::ulnclaw_home().join("config.toml"));
            let content = toml::to_string_pretty(&updated)
                .map_err(|e| format!("serialize config: {}", e))?;
            std::fs::write(&path, content)
                .map_err(|e| format!("write {}: {}", path.display(), e))?;
            println!("Deleted MoA preset: {}", name);
            print_moa_presets(&updated);
        }
    }
    Ok(())
}

fn tools_cmd(config: &UlncLawConfig) -> Result<(), String> {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    toolsets::apply_toolset_policy(&mut registry, &config.enabled_toolsets, &config.disabled_toolsets);
    println!("Toolsets:");
    for name in toolsets::toolsets().keys() {
        println!("  {}", name);
    }
    println!("\nEnabled tools ({}):", registry.len());
    for def in registry.definitions() {
        println!("  {}", def.name);
    }
    Ok(())
}

async fn skills_cmd(action: SkillAction) -> Result<(), String> {
    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    let dir = home.join("skills");
    match action {
        SkillAction::List => {
            let skills = ulnclaw::skills::list_skills(&dir);
            if skills.is_empty() {
                println!("No skills installed ({}).", dir.display());
            }
            for skill in skills {
                let skill_md = std::fs::read_to_string(skill.path.join("SKILL.md")).unwrap_or_default();
                let blueprint_note = match ulnclaw::skills::blueprint::parse_blueprint(&skill_md) {
                    Ok(Some(spec)) => format!("  ⏰ {}", spec.schedule),
                    _ => String::new(),
                };
                println!("  {} — {}{}", skill.name, skill.description, blueprint_note);
            }
        }
        SkillAction::Blueprints => {
            let mut found = false;
            for skill in ulnclaw::skills::list_skills(&dir) {
                let content =
                    std::fs::read_to_string(skill.path.join("SKILL.md")).unwrap_or_default();
                let Ok(Some(spec)) = ulnclaw::skills::blueprint::parse_blueprint(&content) else {
                    continue;
                };
                found = true;
                println!("  {} — schedule: {}", skill.name, spec.schedule);
                if spec.deliver != "origin" {
                    println!("    deliver: {}", spec.deliver);
                }
                if let Some(ref prompt) = spec.prompt {
                    println!("    prompt: {}", prompt);
                }
            }
            if !found {
                println!("No blueprint skills installed (add `metadata.hermes.blueprint.schedule` to a SKILL.md frontmatter).");
            }
        }
        SkillAction::Schedule { name, job_name } => {
            let Some(spec) = ulnclaw::skills::blueprint::blueprint_spec_for_installed(&dir, &name)
            else {
                return Err(format!(
                    "skill '{}' is not an installed blueprint (no metadata.hermes.blueprint block)",
                    name
                ));
            };
            let job = ulnclaw::skills::blueprint::blueprint_to_job(&spec, job_name.as_deref())
                .map_err(|e| e.to_string())?;
            let store = ulnclaw::cron::CronStore::open(&home.join("state.db"))
                .map_err(|e| e.to_string())?;
            store.add(&job).map_err(|e| e.to_string())?;
            println!(
                "scheduled blueprint '{}' as job '{}' ({})",
                name, job.name, job.schedule
            );
        }
        SkillAction::Unschedule { name } => {
            let store = ulnclaw::cron::CronStore::open(&home.join("state.db"))
                .map_err(|e| e.to_string())?;
            let wanted = format!("blueprint:{}", name);
            let jobs = store.list().map_err(|e| e.to_string())?;
            let matches: Vec<_> = jobs
                .into_iter()
                .filter(|job| job.name == wanted)
                .collect();
            if matches.is_empty() {
                return Err(format!("no cron job named '{}' found", wanted));
            }
            for job in matches {
                store.remove(&job.id).map_err(|e| e.to_string())?;
                println!("removed job {} ({})", job.id, job.name);
            }
        }
        SkillAction::View { name } => {
            let Some(skill) = ulnclaw::skills::find_skill(&dir, &name) else {
                return Err(format!("skill '{}' not found", name));
            };
            let content = std::fs::read_to_string(skill.path.join("SKILL.md")).map_err(|e| e.to_string())?;
            println!("{}", content);
            for file in ulnclaw::skills::linked_files(&skill.path) {
                println!("  + {}", file);
            }
        }
    }
    Ok(())
}

async fn cron_cmd(action: CronAction) -> Result<(), String> {
    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    let store = ulnclaw::cron::CronStore::open(&home.join("state.db")).map_err(|e| e.to_string())?;
    match action {
        CronAction::List => {
            let jobs = store.list().map_err(|e| e.to_string())?;
            if jobs.is_empty() {
                println!("No cron jobs.");
            }
            for job in jobs {
                println!(
                    "{}  [{}]  {}  next={:?}  enabled={}",
                    job.id, job.schedule, job.name, job.next_run, job.enabled
                );
            }
        }
        CronAction::Remove { id } => {
            store.remove(&id).map_err(|e| e.to_string())?;
            println!("removed {}", id);
        }
        CronAction::Pause { id } => {
            let Some(mut job) = store.get(&id).map_err(|e| e.to_string())? else {
                return Err(format!("job '{}' not found", id));
            };
            job.enabled = false;
            store.update(&job).map_err(|e| e.to_string())?;
            println!("paused {}", id);
        }
        CronAction::Resume { id } => {
            let Some(mut job) = store.get(&id).map_err(|e| e.to_string())? else {
                return Err(format!("job '{}' not found", id));
            };
            job.enabled = true;
            if let Ok(schedule) = ulnclaw::cron::parse_schedule(&job.schedule) {
                job.next_run = ulnclaw::cron::next_run(&schedule);
            }
            store.update(&job).map_err(|e| e.to_string())?;
            println!("resumed {}", id);
        }
    }
    Ok(())
}
