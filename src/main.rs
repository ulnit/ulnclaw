//! ulnclaw CLI — port of the hermes CLI core (chat REPL, one-shot runs,
//! session/skill/cron/tool management).

use clap::{Parser, Subcommand};
use std::io::Write;
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
}

#[derive(Subcommand)]
enum SkillAction {
    List,
    View { name: String },
}

#[derive(Subcommand)]
enum CronAction {
    List,
    Remove { id: String },
    Pause { id: String },
    Resume { id: String },
}

fn load_config(cli: &Cli) -> UlncLawConfig {
    let mut config = UlncLawConfig::load(cli.config.as_ref().map(std::path::Path::new))
        .unwrap_or_default();
    if let Some(ref profile) = cli.profile {
        config = config.with_profile(profile);
    }
    config
}

fn build_provider(config: &UlncLawConfig) -> Result<Arc<OpenAiProvider>, String> {
    let mut builder = OpenAiProvider::builder()
        .endpoint(&config.resolve_base_url())
        .model(&config.model.model)
        .name(&config.model.provider);
    // Local providers (ollama, llama.cpp) run keyless.
    if let Some(api_key) = config.resolve_api_key() {
        builder = builder.api_key(&api_key);
    } else if !matches!(config.model.provider.as_str(), "ollama" | "llamacpp" | "llama_cpp" | "local") {
        return Err("No API key found. Set OPENAI_API_KEY (or api_key in config.toml).".into());
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
        Commands::Init => {
            let path = UlncLawConfig::write_default_if_missing().map_err(|e| e.to_string())?;
            println!("config written to {}", path.display());
            Ok(())
        }
    }
}

async fn make_agent(config: &UlncLawConfig, interactive: bool) -> Result<Arc<Agent>, String> {
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
        context = context.with_approve(Arc::new(|prompt| {
            Box::pin(async move {
                println!("\n⚠️  {}\nApprove? [y/N]", prompt);
                print!("> ");
                std::io::stdout().flush().ok();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).ok();
                matches!(line.trim(), "y" | "Y" | "yes" | "YES")
            })
        }));
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
        ..Default::default()
    });
    let agent = agent.with_store(store).with_tool_context(context);
    let agent = Arc::new(agent);
    // Wire the runners now that the agent is in an Arc.
    agent.wire_runners();
    Ok(agent)
}

async fn one_shot(config: &UlncLawConfig, prompt: &str) -> Result<(), String> {
    let agent = make_agent(config, false).await?;
    let result = agent.run(prompt, None).await.map_err(|e| e.to_string())?;
    println!("{}", result.content);
    Ok(())
}

async fn chat_repl(config: &UlncLawConfig) -> Result<(), String> {
    let agent = make_agent(config, true).await?;
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
                "Commands:\n  /new            start a fresh conversation\n  /history        show turn count\n  /search <text>  search past sessions\n  /tools          list enabled tools\n  /skills         list skills\n  /memory         show persistent memory\n  /sessions       list recent sessions\n  /usage          token usage of this conversation\n  /quit           exit"
            );
        }
        "/history" => {
            println!("{} messages in current conversation.", history.len());
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
        other => println!("unknown command: {} (/help for a list)", other),
    }
    Ok(true)
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
                println!("  {} — {}", skill.name, skill.description);
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
