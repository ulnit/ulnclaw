//! ulnclaw CLI — port of the hermes CLI core (chat REPL, one-shot runs,
//! session/skill/cron/tool management).

use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Write};
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
    /// Multi-provider model catalog (models.dev registry)
    Models {
        #[command(subcommand)]
        action: Option<ModelsAction>,
    },
    /// Persistent memory: status, or `reset` to erase (hermes memory)
    Memory {
        /// Omit for status; `reset [all|memory|user]` erases memory files
        args: Vec<String>,
        /// Skip the interactive confirmation (reset)
        #[arg(long)]
        yes: bool,
    },
    /// Skill library curation — pin/archive/restore/prune/usage reports
    /// (hermes `hermes curator`)
    Curator {
        #[command(subcommand)]
        action: Option<CuratorAction>,
    },
    /// What Hermes has learned, on a timeline — learned skills & memories
    /// (hermes `hermes journey`)
    Journey {
        #[command(subcommand)]
        action: Option<JourneyAction>,
        /// Render the timeline built up to this point (0=oldest, 1=now)
        #[arg(long, default_value = "1.0")]
        reveal: f64,
        /// Animate the build-up over time (Ctrl-C to stop)
        #[arg(long)]
        play: bool,
        /// Animation frames per second for --play (default 12)
        #[arg(long, default_value = "12")]
        fps: u32,
        /// Override render width in columns
        #[arg(long)]
        width: Option<usize>,
        /// Override render height in rows
        #[arg(long)]
        height: Option<usize>,
        /// Disable color output
        #[arg(long)]
        no_color: bool,
        /// Print the raw graph payload as JSON and exit
        #[arg(long)]
        json: bool,
    },
    /// Write a default config.toml
    Init,
    /// List available skins/themes (hermes skin engine)
    Skins,
    /// Suggested automations — list/accept/dismiss pending cron proposals
    /// (hermes /suggestions)
    Suggestions {
        /// Subcommand args (accept N | dismiss N | catalog | clear)
        args: Vec<String>,
    },
    /// Usage insights and analytics over session history (hermes insights)
    Insights {
        /// Number of days to analyze
        #[arg(long, default_value = "30")]
        days: u32,
        /// Filter by source (cli, gateway, cron, …)
        #[arg(long)]
        source: Option<String>,
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Terminal approval mode: show, or persist manual|smart|off (hermes approvals)
    Approvals {
        /// Mode to persist: manual | smart | off (omit to show current)
        mode: Option<String>,
    },
    /// Measure the system prompt + tool-schema payload (hermes prompt-size)
    PromptSize {
        /// Emit the breakdown as JSON
        #[arg(long)]
        json: bool,
    },
    /// Diagnostics: collect a redacted share bundle (hermes debug)
    Debug {
        /// Bundle subcommand (currently: report)
        #[arg(default_value = "report")]
        action: String,
        /// Lines of recent log to include per file
        #[arg(long, default_value = "200")]
        lines: usize,
        /// Include raw (unredacted) log content — handle with care
        #[arg(long)]
        no_redact: bool,
        /// Output directory (default: ulnclaw-debug-<timestamp>)
        #[arg(long)]
        output: Option<String>,
    },
    /// Diagnose configuration and dependencies (hermes doctor)
    Doctor {
        /// Attempt to fix issues automatically
        #[arg(long)]
        fix: bool,
        /// Probe the configured provider API endpoint
        #[arg(long)]
        online: bool,
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show status of all components (hermes status)
    Status {
        /// Show all details (redacted for sharing; default rendering already redacts)
        #[arg(long)]
        all: bool,
        /// Run deep checks (may take longer)
        #[arg(long)]
        deep: bool,
    },
    /// Generate shell completion scripts (hermes completion)
    Completion {
        /// Shell to generate completions for (bash, zsh, fish, elvish, powershell)
        shell: clap_complete::Shell,
    },
    /// Dump a compact, copy-pasteable setup summary (hermes dump)
    Dump {
        /// Show redacted key values instead of set/not set
        #[arg(long)]
        show_keys: bool,
    },
    /// Show version, install info and update status (hermes version)
    Version {
        /// Skip the online update check
        #[arg(long)]
        no_update_check: bool,
    },
    /// View and edit configuration (hermes config)
    Config {
        /// Subcommand: show (default) | get <key> | set <key> <value> | unset <key> | path | env-path | edit
        args: Vec<String>,
        /// Print the value as JSON (config get)
        #[arg(long)]
        json: bool,
        /// Skip the unknown-key notice (config set)
        #[arg(long)]
        force: bool,
    },
    /// Manage the fallback provider chain (hermes fallback)
    Fallback {
        /// Subcommand: list (default) | add <provider:model> | remove <N|provider:model> | clear
        args: Vec<String>,
        /// Assume yes (skip the clear confirmation)
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Back up the ulnclaw home directory to a zip (hermes backup)
    Backup {
        /// Snapshot management: list | restore <id> | prune [keep]
        action: Vec<String>,
        /// Output path for the zip (default: ~/ulnclaw-backup-<timestamp>.zip)
        #[arg(short, long)]
        output: Option<String>,
        /// Quick snapshot: only critical state files (config, state.db, .env, cron)
        #[arg(short, long)]
        quick: bool,
        /// Label for the snapshot (only used with --quick)
        #[arg(short, long)]
        label: Option<String>,
    },
    /// Restore from a backup zip, overlaying onto the current home (hermes import)
    Import {
        /// Path to the backup zip
        zip: String,
    },
    /// Update ulnclaw to the latest version (hermes update)
    Update {
        /// Check whether an update is available without installing anything
        #[arg(long)]
        check: bool,
        /// Update against this branch instead of the current one
        #[arg(long)]
        branch: Option<String>,
        /// Assume yes for interactive prompts (ulnclaw flow is non-interactive)
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// View and filter ulnclaw log files (hermes logs)
    Logs {
        /// Log to view: agent (default), errors, gateway, or 'list'
        log_name: Option<String>,
        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
        /// Follow the log in real time (like tail -f)
        #[arg(short = 'f', long)]
        follow: bool,
        /// Minimum log level (DEBUG, INFO, WARNING, ERROR, CRITICAL)
        #[arg(long)]
        level: Option<String>,
        /// Filter lines containing this session ID substring
        #[arg(long)]
        session: Option<String>,
        /// Show lines since TIME ago (e.g. 1h, 30m, 2d)
        #[arg(long)]
        since: Option<String>,
        /// Filter by component: gateway, agent, tools, cli, cron, browser
        #[arg(long)]
        component: Option<String>,
    },
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
    /// Prune (delete) ended sessions matching filters (hermes sessions prune)
    Prune {
        #[command(flatten)]
        filters: SessionFilterArgs,
        /// Also prune archived sessions (default: skip them)
        #[arg(long)]
        include_archived: bool,
        /// Show candidates without deleting anything
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Archive (soft-hide, recoverable) ended sessions matching filters
    /// (hermes sessions archive)
    Archive {
        #[command(flatten)]
        filters: SessionFilterArgs,
        /// Show candidates without archiving anything
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Session stats: totals, per-source counts, database size
    /// (hermes sessions stats)
    Stats,
}

/// Shared time/filter flags for `sessions prune` / `sessions archive`
/// (hermes session_filters surface). Time values accept durations (`5h`,
/// `30m`, `2d`, `1w`, bare number = days) or ISO timestamps
/// (`2026-07-05`, `2026-07-05 14:30`).
#[derive(clap::Args)]
struct SessionFilterArgs {
    /// Last activity older than this (duration or ISO timestamp)
    #[arg(long)]
    older_than: Option<String>,
    /// Last activity newer than this (duration or ISO timestamp)
    #[arg(long)]
    newer_than: Option<String>,
    /// Session started before this (duration or ISO timestamp)
    #[arg(long)]
    before: Option<String>,
    /// Session started after this (duration or ISO timestamp)
    #[arg(long)]
    after: Option<String>,
    /// Exact source match (cli, cron, gateway, ...)
    #[arg(long)]
    source: Option<String>,
    /// Case-insensitive substring match on the session title
    #[arg(long)]
    title: Option<String>,
    /// Exact end_reason match (e.g. compression, ended)
    #[arg(long)]
    end_reason: Option<String>,
    /// Session cwd under this directory prefix
    #[arg(long)]
    cwd: Option<String>,
    /// Minimum message count
    #[arg(long)]
    min_messages: Option<i64>,
    /// Maximum message count
    #[arg(long)]
    max_messages: Option<i64>,
    /// Case-insensitive substring match on the model id
    #[arg(long)]
    model: Option<String>,
    /// Minimum total tokens (input + output)
    #[arg(long)]
    min_tokens: Option<i64>,
    /// Maximum total tokens (input + output)
    #[arg(long)]
    max_tokens: Option<i64>,
}

impl SessionFilterArgs {
    /// Translate CLI flags into prune filters (hermes `build_prune_filters`).
    fn build(&self) -> Result<ulnclaw::session::filters::PruneFilters, String> {
        use ulnclaw::session::filters::{parse_point_in_time, PruneFilters};
        let mut filters = PruneFilters::default();
        if let Some(value) = &self.older_than {
            let bound = parse_point_in_time(value, "--older-than")?;
            filters.last_active_before = Some(match filters.last_active_before {
                Some(current) => current.min(bound),
                None => bound,
            });
        }
        if let Some(value) = &self.newer_than {
            let bound = parse_point_in_time(value, "--newer-than")?;
            filters.last_active_after = Some(match filters.last_active_after {
                Some(current) => current.max(bound),
                None => bound,
            });
        }
        if let Some(value) = &self.before {
            let bound = parse_point_in_time(value, "--before")?;
            filters.started_before = Some(match filters.started_before {
                Some(current) => current.min(bound),
                None => bound,
            });
        }
        if let Some(value) = &self.after {
            let bound = parse_point_in_time(value, "--after")?;
            filters.started_after = Some(match filters.started_after {
                Some(current) => current.max(bound),
                None => bound,
            });
        }
        if let Some(started_after) = filters.started_after {
            if let Some(started_before) = filters.started_before {
                if started_after >= started_before {
                    return Err(format!(
                        "Empty start-time window: the --after bound ({}) is not earlier than the --before bound ({})",
                        ulnclaw::session::filters::format_epoch(Some(started_after)),
                        ulnclaw::session::filters::format_epoch(Some(started_before))
                    ));
                }
            }
        }
        filters.source = self.source.clone();
        filters.title_like = self.title.clone();
        filters.end_reason = self.end_reason.clone();
        filters.cwd_prefix = self.cwd.clone();
        filters.min_messages = self.min_messages;
        filters.max_messages = self.max_messages;
        filters.model_like = self.model.clone();
        filters.min_tokens = self.min_tokens;
        filters.max_tokens = self.max_tokens;
        Ok(filters)
    }
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
enum ModelsAction {
    /// List catalog providers (id, name, model count, env vars)
    Providers,
    /// List models for a provider from the catalog (agentic by default)
    List {
        provider: String,
        /// Include non-agentic models (TTS, embeddings, image, ...)
        #[arg(long)]
        all: bool,
        /// Force a catalog refresh before listing
        #[arg(long)]
        refresh: bool,
    },
    /// Show metadata for one model (limits, cost, capabilities)
    Info { provider: String, model: String },
    /// Force-refresh the local models.dev cache
    Refresh,
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
    /// Security-scan an installed skill (hermes skills_guard)
    Scan {
        name: String,
        /// Source identifier for trust resolution (e.g. openai/skills)
        #[arg(long, default_value = "community")]
        source: String,
        /// Emit the scan result as JSON
        #[arg(long)]
        json: bool,
        /// Evaluate the install policy with force override
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum CuratorAction {
    /// Skill usage summary (states, provenance, pins, unmanaged)
    Status,
    /// Pin a skill so auto-transitions never touch it
    Pin { skill: String },
    /// Unpin a skill
    Unpin { skill: String },
    /// Archive a skill now (recoverable via restore)
    Archive { skill: String },
    /// Restore an archived skill
    Restore { skill: String },
    /// List archived (recoverable) skills
    ListArchived,
    /// Usage telemetry table for every skill on disk
    Usage {
        /// Sort order: activity (default), name, or recent
        #[arg(long, default_value = "activity")]
        sort: String,
        /// Emit rows as JSON
        #[arg(long)]
        json: bool,
    },
    /// Bulk-archive unpinned agent-created skills idle for >= N days
    Prune {
        /// Idle threshold in days
        #[arg(long, default_value = "90")]
        days: u64,
        /// Preview without archiving
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Hand unmanaged skills to the curator (stamps provenance)
    Adopt {
        /// Skill name(s) to adopt
        skill: Vec<String>,
        /// Adopt every unmanaged skill
        #[arg(long)]
        all_unmanaged: bool,
        /// Preview without changing anything
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// List skills with no provenance marker
    ListUnmanaged,
}

#[derive(Subcommand)]
enum JourneyAction {
    /// List node ids (for delete/edit)
    List { #[arg(long)] no_color: bool },
    /// Delete a learned skill (archived) or memory by node id
    Delete {
        /// Node id (skill name or memory:<source>:<index>; see `journey list`)
        node: String,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Edit a learned skill or memory by node id in $EDITOR
    Edit {
        /// Node id (skill name or memory:<source>:<index>; see `journey list`)
        node: String,
    },
}

#[derive(Subcommand)]
enum CronAction {
    List,
    Remove { id: String },
    Pause { id: String },
    Resume { id: String },
    /// Run a job once, immediately (unattended: cron approval mode applies)
    Run { id: String },
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
    // Resolve the active theme once per process (hermes init_skin_from_config).
    ulnclaw::skin::init_skin_from_config(&config);
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
    // Rust blocks SIGPIPE by default; restore the default handler so piping
    // into `head` (e.g. `ulnclaw completion bash | head`) exits silently
    // instead of panicking on a closed stdout.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    init_logging(cli.verbose);

    let config = load_config(&cli);
    let result = dispatch(cli, config).await;
    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

/// Build one full gateway stack (approval router + agent + state + cron)
/// rooted at `home`. Used for the default gateway and for each multiplex
/// `/p/<profile>` mirror (profile-scoped home).
async fn build_gateway_stack(
    config: &UlncLawConfig,
    home: &std::path::Path,
    gateway_key: Option<String>,
) -> Result<Arc<ulnclaw::gateway::GatewayState>, String> {
    std::fs::create_dir_all(home).ok();
    let router = ulnclaw::gateway::ApprovalRouter::with_options(
        std::time::Duration::from_secs(config.approvals.timeout),
        Some(home.join("approvals.json")),
    );
    let state_holder: Arc<tokio::sync::OnceCell<Arc<ulnclaw::gateway::GatewayState>>> =
        Arc::new(tokio::sync::OnceCell::new());
    let approve = ulnclaw::gateway::gateway_approve_fn(router.clone(), state_holder.clone());
    let agent = make_agent_in(config, false, Some(approve), home).await?;
    agent.context().set_async_delivery(true);
    let state = ulnclaw::gateway::GatewayState::new(
        agent,
        config.model.model.clone(),
        config.model.provider.clone(),
        gateway_key,
        router,
    )
    .map_err(|e| e.to_string())?;
    state_holder.set(state.clone()).ok();
    let cron_store =
        ulnclaw::cron::CronStore::open(&home.join("state.db")).map_err(|e| e.to_string())?;
    state.cron.set(std::sync::Arc::new(cron_store)).ok();
    state.skills_dir.set(home.join("skills")).ok();
    // Cron scheduler: dispatch due jobs as tracked cron runs (hermes
    // scheduler loop). 30s polling matches the job-timing granularity.
    ulnclaw::gateway::spawn_cron_scheduler(state.clone(), 30);
    Ok(state)
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
    let state = build_gateway_stack(config, &home, gateway.key.clone()).await?;

    // `/p/<profile>` multiplexing (hermes api_server parity): every route
    // is mirrored under `/p/<profile>/...`. With `[gateway]
    // multiplex_profiles = true` each mirror is backed by its own stack
    // built from the `[profiles.<name>]` override; otherwise the prefix is
    // accepted and served by the default profile.
    let hub = {
        let multiplex = gateway.multiplex_profiles;
        let profiles: std::collections::HashSet<String> =
            config.profiles.keys().cloned().collect();
        let base_config = config.clone();
        let base_home = home.clone();
        let gateway_key = gateway.key.clone();
        let builder: ulnclaw::gateway::ProfileRouterBuilder = Arc::new(move |name: String| {
            let config = base_config.clone();
            let home = base_home.clone();
            let key = gateway_key.clone();
            Box::pin(async move {
                let profile_config = config.with_profile(&name);
                let profile_home = home.join("profiles").join(&name);
                let state = build_gateway_stack(&profile_config, &profile_home, key)
                    .await
                    .map_err(|e| {
                        ulnclaw::error::AgentError::config(format!(
                            "profile '{}' gateway stack: {}",
                            name, e
                        ))
                    })?;
                Ok(ulnclaw::gateway::router(state))
            })
        });
        ulnclaw::gateway::ProfileHub::new(
            multiplex,
            profiles,
            ulnclaw::gateway::router(state.clone()),
            builder,
        )
    };

    ulnclaw::gateway::serve_multiplex(state, Some(hub), &gateway.host, gateway.port)
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
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::Layer;
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
    };
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter);
    // hermes_logging parity: rotating file handlers under <home>/logs/
    // (agent.log INFO+, errors.log WARNING+, gateway.log gateway targets).
    let file_layers = ulnclaw::logs::file_layers();
    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layers)
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
        Commands::Cron { action } => cron_cmd(&config, action.unwrap_or(CronAction::List)).await,
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
        Commands::Models { action } => {
            // reqwest's blocking client builds its own runtime, so catalog
            // network fetches must run off the async main context.
            tokio::task::spawn_blocking(move || {
                models_cmd(action.unwrap_or(ModelsAction::Providers))
            })
            .await
            .map_err(|e| e.to_string())?
        }
        Commands::Memory { args, yes } => {
            let home = ulnclaw::config::ulnclaw_home();
            ulnclaw::memory_cmd::handle_memory_command(&home, &args, yes)
        }
        Commands::Curator { action } => {
            curator_cmd(action.unwrap_or(CuratorAction::Status))
        }
        Commands::Journey {
            action,
            reveal,
            play,
            fps,
            width,
            height,
            no_color,
            json,
        } => journey_cmd(action, reveal, play, fps, width, height, no_color, json),
        Commands::Init => {
            let path = UlncLawConfig::write_default_if_missing().map_err(|e| e.to_string())?;
            println!("config written to {}", path.display());
            Ok(())
        }
        Commands::Skins => {
            let active = ulnclaw::skin::get_active_skin_name();
            for info in ulnclaw::skin::list_skins() {
                let marker = if info.name == active { "*" } else { " " };
                println!(
                    "{} {:<16} {:<8} {}",
                    marker, info.name, info.source, info.description
                );
            }
            println!("Active skin: {} (set [display] skin in config.toml)", active);
            Ok(())
        }
        Commands::Suggestions { args } => {
            println!("{}", ulnclaw::cron::suggestions::handle_suggestions_command(&args.join(" ")));
            Ok(())
        }
        Commands::Insights { days, source, json } => {
            let engine = ulnclaw::insights::InsightsEngine::open_default().map_err(|e| e.to_string())?;
            let report = engine
                .generate(days, source.as_deref(), Some(config.model.provider.as_str()))
                .map_err(|e| e.to_string())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
            } else {
                print!("{}", ulnclaw::insights::format_terminal(&report));
            }
            Ok(())
        }
        Commands::Approvals { mode } => {
            let result = ulnclaw::approvals_cmd::run_approval_mode_command(mode.as_deref());
            if result.ok {
                println!("{}", result.message);
                Ok(())
            } else {
                Err(result.message)
            }
        }
        Commands::Debug { action, lines, no_redact, output } => {
            if action != "report" {
                return Err(format!("unknown debug action '{action}' (expected: report)"));
            }
            let report = ulnclaw::debug_cmd::handle_debug_command(
                &config,
                cli.profile.as_deref(),
                lines,
                !no_redact,
                output.as_deref(),
            )?;
            print!("{report}");
            Ok(())
        }
        Commands::PromptSize { json } => {
            let home = ulnclaw::config::ulnclaw_home();
            let mut registry = ulnclaw::tools::ToolRegistry::new();
            ulnclaw::tools::builtin::register_builtin_tools(&mut registry);
            ulnclaw::toolsets::apply_toolset_policy(
                &mut registry,
                &config.enabled_toolsets,
                &config.disabled_toolsets,
            );
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let data =
                ulnclaw::prompt_size::compute_prompt_breakdown(&config, &home, &cwd, &registry);
            if json {
                println!("{}", serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?);
            } else {
                print!("{}", ulnclaw::prompt_size::render_breakdown(&data));
            }
            Ok(())
        }
        Commands::Doctor { fix, online, json } => {
            let opts = ulnclaw::doctor::DoctorOptions { fix, online, json };
            let report = ulnclaw::doctor::run_doctor(&config, &opts);
            if json {
                println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
            } else {
                print!("{}", report.render());
            }
            Ok(())
        }
        Commands::Status { all: _, deep } => {
            let opts = ulnclaw::status::StatusOptions { deep };
            print!("{}", ulnclaw::status::show_status(&config, &opts));
            Ok(())
        }
        Commands::Completion { shell } => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            clap_complete::generate(shell, &mut cmd, "ulnclaw", &mut std::io::stdout());
            Ok(())
        }
        Commands::Dump { show_keys } => {
            print!(
                "{}",
                ulnclaw::dump::build_dump(&config, cli.profile.as_deref(), show_keys)
            );
            Ok(())
        }
        Commands::Version { no_update_check } => {
            let root = ulnclaw::update::find_repo_root();
            print!(
                "{}",
                ulnclaw::dump::build_version_report(root.as_deref(), !no_update_check)
            );
            Ok(())
        }
        Commands::Config { args, json, force } => {
            let out = ulnclaw::config_cmd::handle_config_command(&args, json, force)?;
            if !out.is_empty() {
                println!("{out}");
            }
            Ok(())
        }
        Commands::Fallback { args, yes } => {
            let home = ulnclaw::config::ulnclaw_home();
            print!("{}", ulnclaw::fallback::handle_fallback_command(&home, &args, yes)?);
            Ok(())
        }
        Commands::Backup { action, output, quick, label } => {
            let home = ulnclaw::config::ulnclaw_home();
            match action.first().map(|a| a.as_str()) {
                Some("list") => {
                    let snapshots = ulnclaw::backup::list_quick_snapshots(&home);
                    if snapshots.is_empty() {
                        println!("No quick snapshots yet. Create one with 'ulnclaw backup --quick'.");
                    } else {
                        println!("Quick snapshots in {}/{}:", home.display(), ulnclaw::backup::QUICK_SNAPSHOTS_DIR);
                        for snapshot in &snapshots {
                            println!(
                                "  {:<32} {:>4} file(s)  {:>10}",
                                snapshot.id,
                                snapshot.files,
                                ulnclaw::backup::format_size(snapshot.bytes as f64)
                            );
                        }
                    }
                    Ok(())
                }
                Some("restore") => {
                    let Some(id) = action.get(1) else {
                        return Err("usage: ulnclaw backup restore <snapshot-id>".into());
                    };
                    match ulnclaw::backup::restore_quick_snapshot(&home, id) {
                        Ok(true) => println!("✓ Restored state from snapshot {id}."),
                        Ok(false) => println!("Snapshot '{id}' not found or empty."),
                        Err(e) => return Err(e),
                    }
                    Ok(())
                }
                Some("prune") => {
                    let keep: usize = action
                        .get(1)
                        .and_then(|k| k.parse().ok())
                        .unwrap_or(ulnclaw::backup::QUICK_DEFAULT_KEEP);
                    let removed = ulnclaw::backup::prune_quick_snapshots(&home, keep);
                    println!("Pruned {removed} snapshot(s) (keeping {keep}).");
                    Ok(())
                }
                Some(unknown) => Err(format!(
                    "Unknown backup action: '{unknown}'. Use list, restore <id>, or prune [keep]."
                )),
                None => {
                    if quick {
                        match ulnclaw::backup::create_quick_snapshot(&home, label.as_deref(), None, None)? {
                            Some(id) => {
                                println!("✓ Quick snapshot created: {id}");
                                println!("  Restore with: ulnclaw backup restore {id}");
                            }
                            None => println!("No state files found to snapshot."),
                        }
                        Ok(())
                    } else {
                        println!("Scanning {} ...", home.display());
                        let summary = ulnclaw::backup::create_backup(
                            &home,
                            output.as_ref().map(std::path::Path::new),
                        )?;
                        println!("Backing up {} files ...", summary.file_count);
                        print!("{}", ulnclaw::backup::format_backup_summary(&summary));
                        Ok(())
                    }
                }
            }
        }
        Commands::Import { zip } => {
            let home = ulnclaw::config::ulnclaw_home();
            let zip_path = std::path::PathBuf::from(&zip);
            // Safety net snapshot of current state before overlaying.
            if let Some(id) = ulnclaw::backup::create_quick_snapshot(&home, Some("pre-import"), None, None)? {
                println!("→ Current state snapshotted as {id} before import.");
            }
            let report = ulnclaw::backup::import_backup(&home, &zip_path)?;
            print!("{}", ulnclaw::backup::format_import_report(&report));
            if let Some(message) = ulnclaw::backup::restore_cron_jobs_if_emptied(&home) {
                println!("{message}");
            }
            Ok(())
        }
        Commands::Update { check, branch, yes } => {
            let opts = ulnclaw::update::UpdateOptions { check, branch, yes };
            let root = ulnclaw::update::find_repo_root()
                .ok_or("✗ Not a git repository — cannot check for updates.")?;
            if check {
                let (outcome, log_lines) = ulnclaw::update::check_update(&root, &opts)?;
                for line in log_lines {
                    println!("{line}");
                }
                print!("{}", ulnclaw::update::format_check_report(&outcome));
            } else {
                // hermes _run_pre_update_backup: quick state snapshot first.
                let home = ulnclaw::config::ulnclaw_home();
                if let Some(id) = ulnclaw::backup::create_pre_update_backup(&home) {
                    println!("→ Pre-update state snapshot: {id}");
                }
                let report = ulnclaw::update::apply_update(&root, &opts)?;
                print!("{}", ulnclaw::update::format_update_report(&report));
                if let Some(message) = ulnclaw::backup::restore_cron_jobs_if_emptied(&home) {
                    println!("{message}");
                }
            }
            Ok(())
        }
        Commands::Logs { log_name, lines, follow, level, session, since, component } => {
            let name = log_name.unwrap_or_else(|| "agent".to_string());
            if name == "list" {
                print!("{}", ulnclaw::logs::list_logs());
                return Ok(());
            }
            let opts = ulnclaw::logs::TailOptions {
                num_lines: lines,
                follow,
                level,
                session: session.clone(),
                since: since.clone(),
                component: component.clone(),
            };
            print!("{}", ulnclaw::logs::tail_log(&name, &opts)?);
            if follow {
                ulnclaw::logs::follow_log(&name, &opts)?;
            }
            Ok(())
        }
    }
}

async fn make_agent(
    config: &UlncLawConfig,
    interactive: bool,
    approve_override: Option<ulnclaw::tools::context::ApproveFn>,
) -> Result<Arc<Agent>, String> {
    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    make_agent_in(config, interactive, approve_override, &home).await
}

/// Build an agent rooted at an explicit home directory — the multiplex
/// gateway uses this to scope each `/p/<profile>` agent to
/// `<home>/profiles/<name>` (hermes profile home scoping).
async fn make_agent_in(
    config: &UlncLawConfig,
    interactive: bool,
    approve_override: Option<ulnclaw::tools::context::ApproveFn>,
    home: &std::path::Path,
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

    std::fs::create_dir_all(home).ok();
    let store = Arc::new(SqliteSessionStore::open(home.join("state.db")).map_err(|e| e.to_string())?);
    // Crash recovery for background delegations (hermes durable registry):
    // rows still running from a previous process become terminal
    // "outcome unknown" results delivered on the next drain.
    if ulnclaw::async_delegation::recover_from_store(&store) > 0 {
        eprintln!("[delegation] recovered abandoned background delegation(s) from a previous run");
    }

    let mut context = ToolContext::new()
        .with_home(home.to_path_buf())
        .with_config(config.clone())
        .with_async_delivery(interactive)
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

/// Print a random feature tip tinted with the active skin's banner_dim
/// (hermes `✦ Tip:` line). Color only applies on a TTY without NO_COLOR.
fn print_tip() {
    let tip = ulnclaw::tips::format_tip(ulnclaw::tips::get_random_tip());
    let skin = ulnclaw::skin::get_active_skin();
    let color = skin.get_color("banner_dim", "#B8860B");
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        println!("{}", ulnclaw::skin::colorize(&color, &tip));
    } else {
        println!("{}", tip);
    }
}

/// Hermes-style welcome banner: wordmark + skin-colored panel with model,
/// toolsets, skills, and update status. Degrades to plain summary lines
/// when stdout is not a TTY.
async fn print_welcome_banner(config: &UlncLawConfig, agent: &Arc<Agent>) {
    let toolsets: Vec<(String, Vec<String>)> = agent
        .toolset_names()
        .into_iter()
        .map(|toolset| {
            let display = ulnclaw::banner::display_toolset_name(&toolset);
            let tools = agent.toolset_tool_names(&toolset);
            (display, tools)
        })
        .collect();
    let total_tools: usize = toolsets.iter().map(|(_, tools)| tools.len()).sum();
    // The models.dev lookup uses a blocking reqwest client, which must not
    // run (or be dropped) inside the async runtime; spawn_blocking keeps it
    // on a blocking thread. The 2s cap keeps startup snappy when the
    // registry is unreachable — the task keeps running and warms the cache.
    let lookup_provider = config.model.provider.clone();
    let lookup_model = config.model.model.clone();
    let context_length = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::task::spawn_blocking(move || {
            ulnclaw::models_dev::lookup_models_dev_context(&lookup_provider, &lookup_model)
        }),
    )
    .await
    .ok()
    .and_then(|joined| joined.ok())
    .flatten();
    let info = ulnclaw::banner::BannerInfo {
        model: config.model.model.clone(),
        provider: config.model.provider.clone(),
        cwd: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        session_id: Some(agent.context().session_id.clone()),
        context_length,
        toolsets,
        skills: ulnclaw::banner::get_available_skills(),
        total_tools,
        yolo: config.approvals.mode == "off",
        update_behind: ulnclaw::banner::get_update_result(std::time::Duration::from_millis(500)),
    };
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        let term_width = ulnclaw::banner::terminal_width();
        println!("{}", ulnclaw::banner::build_startup_display(&info, term_width));
    } else {
        println!(
            "ulnclaw {} — model: {} ({})",
            ulnclaw::VERSION,
            config.model.model,
            config.model.provider
        );
        println!("Type /help for commands, /quit to exit.");
    }
}

async fn chat_repl(config: &UlncLawConfig) -> Result<(), String> {
    // Kick off the git update check while the agent is being constructed
    // (hermes prefetch_update_check on the startup path).
    ulnclaw::banner::prefetch_update_check();
    let agent = make_agent(config, true, None).await?;
    // Cross-process active-session cap (hermes active_sessions): the lease
    // is released automatically when the REPL exits.
    let (_session_lease, session_limit_error) =
        ulnclaw::active_sessions::try_acquire_active_session(
            &agent.context().session_id,
            "cli",
            config,
        );
    if let Some(message) = session_limit_error {
        return Err(message);
    }
    print_welcome_banner(config, &agent).await;
    // Random feature tip (hermes startup tip), tinted with the active
    // skin's banner_dim.
    print_tip();

    let mut history: Vec<Message> = Vec::new();
    let stdin = std::io::stdin();
    let session_key = agent.context().session_id.clone();
    let store = agent.context().store.clone();
    // Standing-goal (Ralph loop) manager for this session — state persists
    // in state.db keyed by session id (hermes GoalManager per live session).
    let mut goal_manager = ulnclaw::goals::GoalManager::new(
        session_key.clone(),
        store.clone(),
        ulnclaw::goals::DEFAULT_MAX_TURNS,
    );
    // Input queued by slash commands (e.g. /goal kicks the loop with the
    // goal text; the judge feeds continuation prompts through here too).
    let mut pending: Option<String> = None;
    loop {
        // Drain finished background delegations into the conversation
        // (hermes CLI completion drain, positive-ownership by session key).
        for completion in
            ulnclaw::async_delegation::drain_completions(store.as_deref(), &session_key)
        {
            println!(
                "\n[delegation {} finished — consolidated result injected]",
                completion.delegation_id
            );
            history.push(Message {
                role: Role::User,
                content: Some(completion.message),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }
        let input = if let Some(next) = pending.take() {
            next
        } else {
            print!("\n> ");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            stdin.read_line(&mut line).map_err(|e| e.to_string())?;
            line.trim().to_string()
        };
        if input.is_empty() {
            continue;
        }
        if input.starts_with('/') {
            match handle_slash(&input, &agent, &mut history, &mut goal_manager, &mut pending).await {
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
                // The Ralph loop: when a standing goal is active, judge the
                // turn and feed the continuation prompt back as the next
                // user message until the goal is done/paused/cleared.
                if goal_manager.is_active() {
                    if let Some(provider) = agent.tool_context().provider.clone() {
                        let background = ulnclaw::goals::gather_background_processes();
                        let decision = goal_manager
                            .evaluate_after_turn(config, provider, &result.content, &background)
                            .await;
                        if !decision.message.is_empty() {
                            println!("\n{}", decision.message);
                        }
                        if decision.should_continue {
                            if let Some(prompt) = decision.continuation_prompt {
                                pending = Some(prompt);
                            }
                        }
                    }
                }
            }
            Err(e) => println!("error: {}", e),
        }
    }
    Ok(())
}

/// System note injected into the conversation when `/browser connect`
/// succeeds (hermes `_pending_input` context injection).
const BROWSER_CONNECT_NOTE: &str = "[System note: The user invoked /browser connect and connected your browser tools to a Chromium-family dev/debug browser via Chrome DevTools Protocol. Your browser_navigate, browser_snapshot, browser_click, and other browser tools now control that CDP browser. The command itself is a signal that using browser tools for their current browser-related request is expected; do not wait for separate permission just because CDP is connected. This is typically an isolated debug profile, not the user's main everyday browser. It is still user-visible and may contain pages, logged-in sessions, or cookies in that debug profile, so avoid destructive actions, closing tabs, or navigating away unless the user's task calls for it.]";

async fn handle_slash(
    input: &str,
    agent: &Arc<Agent>,
    history: &mut Vec<Message>,
    goals: &mut ulnclaw::goals::GoalManager,
    pending: &mut Option<String>,
) -> Result<bool, String> {
    let mut parts = input.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match cmd {
        "/quit" | "/exit" | "/q" => return Ok(false),
        "/new" => {
            history.clear();
            println!("New conversation started.");
            print_tip();
        }
        "/help" => {
            println!(
                "Commands:\n  /new            start a fresh conversation\n  /history        show turn count\n  /recap          recap recent activity in this conversation\n  /moa <prompt>   one-shot Mixture-of-Agents synthesis (default preset)\n  /search <text>  search past sessions\n  /tools          list enabled tools\n  /browser <status|connect [url]|disconnect>   browser CDP endpoint\n  /skills         list skills\n  /memory         show persistent memory\n  /goal [text|status|show|draft|pause|resume|clear|wait|unwait]   standing goal (Ralph loop)\n  /subgoal [text|remove <n>|clear]   extra criteria on the active goal\n  /suggestions [accept N|dismiss N|catalog|clear]   suggested automations\n  /sessions       list recent sessions\n  /usage          token usage of this conversation\n  /insights [days]  usage analytics across sessions (hermes insights)\n  /rollback [N|hash] [file]   list/restore checkpoints (hermes-style)\n  /rollback diff <N|hash>     preview changes since a checkpoint\n  /diff [N|hash|session]      cumulative session diff / vs a checkpoint\n  /gitdiff [staged|all]     git working-tree diff (what changed here?)\n  /quit           exit"
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
        "/insights" => {
            let days: u32 = rest.parse().unwrap_or(30);
            let provider = agent.tool_context().config.model.provider.clone();
            match ulnclaw::insights::InsightsEngine::open(&ulnclaw::insights::default_store_path()) {
                Ok(engine) => {
                    match engine.generate(days, None, Some(provider.as_str())) {
                        Ok(report) => print!("{}", ulnclaw::insights::format_terminal(&report)),
                        Err(e) => println!("insights failed: {e}"),
                    }
                }
                Err(e) => println!("insights failed: {e}"),
            }
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
        "/browser" => {
            // hermes `/browser` UX: live CDP endpoint control.
            let mut parts = rest.splitn(2, ' ');
            match parts.next().unwrap_or("") {
                "status" | "" => {
                    if ulnclaw::browser::camofox::is_camofox_mode() {
                        let url = ulnclaw::browser::camofox::camofox_url().unwrap_or_default();
                        let available = ulnclaw::browser::camofox::check_available().await;
                        let vnc = ulnclaw::browser::camofox::vnc_url().await;
                        println!(
                            "browser: camofox backend — {url} (available: {available}{})",
                            vnc.map(|v| format!(", vnc: {v}")).unwrap_or_default()
                        );
                    } else {
                        match ulnclaw::browser::endpoint_with_source() {
                            Some((source, raw)) => {
                                let mode = if ulnclaw::browser::is_auto_mode(&raw) { "managed" } else { "endpoint" };
                                println!("browser: configured via {source} — {raw} (mode: {mode})");
                            }
                            None => println!("browser: not configured (set ULNCLAW_BROWSER_CDP or /browser connect <url>, or CAMOFOX_URL for the Camofox backend)"),
                        }
                    }
                }
                "connect" => {
                    let url = parts.next().unwrap_or("").trim().to_string();
                    if url.is_empty() {
                        // Hermes default flow: probe both loopbacks, arbitrate
                        // a squatted port, auto-launch a visible debug
                        // browser with per-candidate diagnostics.
                        let outcome = ulnclaw::browser::connect::connect_local_default(
                            ulnclaw::browser::connect::DEFAULT_BROWSER_CDP_PORT,
                        )
                        .await;
                        println!();
                        for line in &outcome.messages {
                            println!("   {line}");
                        }
                        if let Some(found) = outcome.url {
                            match ulnclaw::browser::set_cdp_override(&found) {
                                Ok(()) => {
                                    println!();
                                    println!("🌐 Browser connected to live Chromium-family browser via CDP");
                                    println!("   Endpoint: {found}");
                                    println!();
                                    history.push(Message {
                                        role: Role::User,
                                        content: Some(BROWSER_CONNECT_NOTE.to_string()),
                                        tool_calls: None,
                                        tool_call_id: None,
                                        name: None,
                                    });
                                }
                                Err(e) => println!("connect failed: {e}"),
                            }
                        }
                    } else {
                        match ulnclaw::browser::set_cdp_override(&url) {
                            Ok(()) => {
                                println!("browser endpoint set to {url} (live until disconnect/exit)");
                                history.push(Message {
                                    role: Role::User,
                                    content: Some(BROWSER_CONNECT_NOTE.to_string()),
                                    tool_calls: None,
                                    tool_call_id: None,
                                    name: None,
                                });
                            }
                            Err(e) => println!("connect failed: {e}"),
                        }
                    }
                }
                "disconnect" => {
                    ulnclaw::browser::clear_cdp_override();
                    println!("browser endpoint override cleared");
                    history.push(Message {
                        role: Role::User,
                        content: Some(
                            "[System note: The user has disconnected the browser tools from their                              live Chromium-family browser. Browser tools are back to default mode                              (managed local browser or configured endpoint).]"
                                .to_string(),
                        ),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });
                }
                other => println!("unknown /browser subcommand: {other} (status|connect|disconnect)"),
            }
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
        "/goal" => {
            // Standing goal — the Ralph loop (hermes /goal). Subcommands:
            // status/show/draft/pause/resume/clear/wait/unwait; anything
            // else is treated as the goal text (inline `field: value` lines
            // become a completion contract).
            let lower = rest.to_ascii_lowercase();
            if rest.is_empty() || lower == "status" {
                println!("  {}", goals.status_line());
            } else if lower == "show" {
                println!("  {}", goals.status_line());
                println!("  {}", goals.render_contract());
            } else if lower.starts_with("draft") {
                let objective = rest["draft".len()..].trim();
                if objective.is_empty() {
                    println!("  Usage: /goal draft <objective in plain language>");
                } else {
                    println!("  Drafting completion contract…");
                    let contract = match agent.tool_context().provider.clone() {
                        Some(provider) => {
                            let config = agent.tool_context().config.clone();
                            ulnclaw::goals::draft_contract(&config, provider, objective).await
                        }
                        None => None,
                    };
                    match goals.set(objective, None, contract) {
                        Ok(state) => {
                            println!("  ⊙ Goal set ({}-turn budget): {}", state.max_turns, state.goal);
                            if state.has_contract() {
                                println!("  Drafted completion contract:");
                                for line in state.contract.render_block().lines() {
                                    println!("    {}", line);
                                }
                                println!(
                                    "  Tighten any field by re-setting the goal with inline lines (e.g. verify: <command>). Use /goal show to review."
                                );
                            } else {
                                println!(
                                    "  Couldn't draft a contract (aux model unavailable) — running as a free-form goal. The per-turn judge still applies."
                                );
                            }
                            pending.replace(state.goal.clone());
                        }
                        Err(e) => println!("  Invalid goal: {}", e),
                    }
                }
            } else if lower == "pause" {
                match goals.pause("user-paused") {
                    Some(state) => println!("  ⏸ Goal paused: {}", state.goal),
                    None => println!("  No goal set."),
                }
            } else if lower == "resume" {
                match goals.resume(true) {
                    Some(state) => {
                        println!("  ▶ Goal resumed: {}", state.goal);
                        println!("  Send any message (or type 'continue') to kick it off.");
                    }
                    None => println!("  No goal to resume."),
                }
            } else if matches!(lower.as_str(), "clear" | "stop" | "done") {
                let had = goals.has_goal();
                goals.clear();
                if had {
                    println!("  ✓ Goal cleared.");
                } else {
                    println!("  No active goal.");
                }
            } else if lower == "wait" || lower.starts_with("wait ") {
                let wait_arg = rest["wait".len()..].trim();
                if wait_arg.is_empty() {
                    println!("  Usage: /goal wait <pid> [reason]");
                } else {
                    let mut wtokens = wait_arg.splitn(2, char::is_whitespace);
                    let pid_str = wtokens.next().unwrap_or("");
                    match pid_str.parse::<u32>() {
                        Ok(pid) => {
                            let reason = wtokens.next().unwrap_or("").trim();
                            match goals.wait_on(pid, reason) {
                                Ok(_) => {
                                    let suffix = if reason.is_empty() { String::new() } else { format!(" ({})", reason) };
                                    println!("  ⏳ Goal parked on pid {}{}. Loop pauses until it exits.", pid, suffix);
                                }
                                Err(e) => println!("  /goal wait: {}", e),
                            }
                        }
                        Err(_) => println!("  /goal wait: <pid> must be an integer process id."),
                    }
                }
            } else if lower == "unwait" {
                if goals.stop_waiting() {
                    println!("  ▶ Wait barrier cleared — goal loop resumes.");
                } else {
                    println!("  No wait barrier set.");
                }
            } else {
                let (headline, contract) = ulnclaw::goals::parse_contract(rest);
                let goal_text = if headline.is_empty() { rest.to_string() } else { headline };
                let contract = if contract.is_empty() { None } else { Some(contract) };
                match goals.set(&goal_text, None, contract) {
                    Ok(state) => {
                        println!("  ⊙ Goal set ({}-turn budget): {}", state.max_turns, state.goal);
                        if state.has_contract() {
                            println!("  Completion contract:");
                            for line in state.contract.render_block().lines() {
                                println!("    {}", line);
                            }
                        }
                        println!(
                            "  After each turn, a judge model checks if the goal is done{}. The agent keeps working until it is, you pause/clear it, or the budget is exhausted. Use /goal status, /goal show, /goal pause, /goal resume, /goal clear.",
                            if state.has_contract() { " against the contract above" } else { "" }
                        );
                        pending.replace(state.goal.clone());
                    }
                    Err(e) => println!("  Invalid goal: {}", e),
                }
            }
        }
        "/subgoal" => {
            // Extra criteria on the active goal (hermes /subgoal).
            if rest.is_empty() {
                println!("  {}", goals.render_subgoals());
            } else {
                let mut sub_parts = rest.splitn(2, ' ');
                match sub_parts.next().unwrap_or("") {
                    "remove" => {
                        let idx_str = sub_parts.next().unwrap_or("").trim();
                        if idx_str.is_empty() {
                            println!("  Usage: /subgoal remove <n>");
                        } else {
                            match idx_str.parse::<usize>() {
                                Ok(n) => match goals.remove_subgoal(n) {
                                    Ok(removed) => println!("  ✓ Removed subgoal {}: {}", n, removed),
                                    Err(e) => println!("  /subgoal remove: {}", e),
                                },
                                Err(_) => println!("  /subgoal remove: <n> must be an integer (1-based index)."),
                            }
                        }
                    }
                    "clear" => match goals.clear_subgoals() {
                        Ok(count) => println!("  ✓ Cleared {} subgoal(s).", count),
                        Err(e) => println!("  /subgoal clear: {}", e),
                    },
                    _ => match goals.add_subgoal(rest) {
                        Ok(text) => println!("  ✓ Added subgoal: {}", text),
                        Err(e) => println!("  /subgoal: {}", e),
                    },
                }
            }
        }
        "/suggestions" => {
            println!("{}", ulnclaw::cron::suggestions::handle_suggestions_command(rest));
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
        SessionAction::Prune { filters, include_archived, dry_run, yes } => {
            let mut filters = filters.build()?;
            // Hermes semantics: a truly bare `sessions prune` (no time
            // window and no filters) means "older than 90 days". ANY
            // filter suppresses the implicit cutoff.
            if filters.is_empty() {
                let cutoff = ulnclaw::session::filters::parse_point_in_time("90", "--older-than")
                    .map_err(|e| e.to_string())?;
                filters.last_active_before = Some(cutoff);
            }
            // Prune skips archived sessions unless --include-archived.
            filters.archived = if include_archived { None } else { Some(false) };
            run_session_prune(&store, filters, true, dry_run, yes)?;
        }
        SessionAction::Archive { filters, dry_run, yes } => {
            let mut filters = filters.build()?;
            if filters.is_empty() {
                return Err(
                    "Refusing to archive every ended session: pass at least one filter                      (e.g. --newer-than 5h, --source cli, --title codex)."
                        .to_string(),
                );
            }
            // Archive only targets not-yet-archived rows (idempotent).
            filters.archived = Some(false);
            run_session_prune(&store, filters, false, dry_run, yes)?;
        }
        SessionAction::Stats => {
            let total = store.count_sessions().map_err(|e| e.to_string())?;
            let messages = store.count_messages().map_err(|e| e.to_string())?;
            println!("Total sessions: {}", total);
            println!("Total messages: {}", messages);
            for (source, count) in store.session_count_by_source().map_err(|e| e.to_string())? {
                println!("  {}: {} sessions", source, count);
            }
            let db_path = home.join("state.db");
            if let Ok(metadata) = std::fs::metadata(&db_path) {
                println!("Database size: {:.1} MB", metadata.len() as f64 / (1024.0 * 1024.0));
            }
        }
    }
    Ok(())
}

/// Shared preview/confirm/execute flow for `sessions prune` and
/// `sessions archive` (hermes sessions_cmd prune/archive branch).
fn run_session_prune(
    store: &SqliteSessionStore,
    filters: ulnclaw::session::filters::PruneFilters,
    delete: bool,
    dry_run: bool,
    yes: bool,
) -> Result<(), String> {
    use ulnclaw::session::filters::format_epoch;
    let candidates = store
        .list_prune_candidates(&filters)
        .map_err(|e| e.to_string())?;
    let verb = if delete { "Delete" } else { "Archive" };
    if candidates.is_empty() {
        println!("No sessions match ({}).", filters.describe());
        return Ok(());
    }
    // Candidates are ordered oldest-activity-first; surface the span so a
    // long-lived but recently used conversation cannot look old merely
    // because of its creation date.
    let oldest = candidates.first().map(|c| c.last_active);
    let newest = candidates.last().map(|c| c.last_active);
    let span = format!(
        "oldest activity {}, newest activity {}",
        format_epoch(oldest),
        format_epoch(newest)
    );
    if dry_run || !yes {
        let shown: Vec<_> = if dry_run { candidates.iter().collect() } else { candidates.iter().take(15).collect() };
        let shown_count = shown.len();
        println!(
            "{} session(s) match ({}; {}):",
            candidates.len(),
            filters.describe(),
            span
        );
        for candidate in shown {
            let title = candidate.title.as_deref().unwrap_or("");
            let title: String = title.chars().take(36).collect();
            let model = candidate
                .model
                .as_deref()
                .unwrap_or("-")
                .rsplit('/')
                .next()
                .unwrap_or("-");
            let model: String = model.chars().take(24).collect();
            println!(
                "  {}  {:<17} {:<10} {:<24} {:>4} msgs  {}",
                candidate.id,
                format_epoch(Some(candidate.last_active)),
                candidate.source,
                model,
                candidate.message_count,
                title
            );
        }
        if candidates.len() > shown_count {
            println!("  … and {} more", candidates.len() - shown_count);
        }
        if dry_run {
            println!(
                "Dry run — nothing {}.",
                if delete { "deleted" } else { "archived" }
            );
            return Ok(());
        }
    }
    if !yes {
        print!("{} these {} session(s) ({})? [y/N] ", verb, candidates.len(), span);
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).ok();
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    let count = if delete {
        store.prune_sessions(&filters).map_err(|e| e.to_string())?
    } else {
        store.archive_sessions(&filters).map_err(|e| e.to_string())?
    };
    if delete {
        println!("Pruned {} session(s).", count);
    } else {
        println!(
            "Archived {} session(s). They're hidden from listings but fully recoverable (nothing was deleted).",
            count
        );
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

// ── curator: skill library curation (hermes hermes_cli/curator.py) ────────

fn curator_cmd(action: CuratorAction) -> Result<(), String> {
    use ulnclaw::{curator, skill_usage};

    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;
    match action {
        CuratorAction::Status => {
            for (label, count) in curator::status_summary(&home) {
                println!("  {:<28} {}", label, count);
            }
            Ok(())
        }
        CuratorAction::Pin { skill } => {
            skill_usage::set_pinned(&home, &skill, true);
            println!("curator: pinned '{}' (will bypass auto-transitions)", skill);
            Ok(())
        }
        CuratorAction::Unpin { skill } => {
            skill_usage::set_pinned(&home, &skill, false);
            println!("curator: unpinned '{}'", skill);
            Ok(())
        }
        CuratorAction::Archive { skill } => {
            if skill_usage::get_record(&home, &skill)
                .get("pinned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err(format!(
                    "'{}' is pinned — unpin first with `ulnclaw curator unpin {}`",
                    skill, skill
                ));
            }
            let (ok, message) = skill_usage::archive_skill(&home, &skill);
            println!("curator: {}", message);
            if ok { Ok(()) } else { Err(message) }
        }
        CuratorAction::Restore { skill } => {
            let (ok, message) = skill_usage::restore_skill(&home, &skill);
            println!("curator: {}", message);
            if ok { Ok(()) } else { Err(message) }
        }
        CuratorAction::ListArchived => {
            let names = skill_usage::list_archived_skill_names(&home);
            if names.is_empty() {
                println!("curator: no archived skills");
                return Ok(());
            }
            for name in names {
                println!("{}", name);
            }
            Ok(())
        }
        CuratorAction::Usage { sort, json } => {
            let mut rows = skill_usage::usage_report(&home);
            match sort.as_str() {
                "name" => rows.sort_by(|a, b| a.name.cmp(&b.name)),
                "recent" => rows.sort_by(|a, b| {
                    b.last_activity_at
                        .clone()
                        .unwrap_or_default()
                        .cmp(&a.last_activity_at.clone().unwrap_or_default())
                }),
                _ => rows.sort_by(|a, b| b.activity_count.cmp(&a.activity_count)),
            }
            if json {
                let payload: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": r.name,
                            "provenance": r.provenance,
                            "use_count": r.use_count,
                            "view_count": r.view_count,
                            "patch_count": r.patch_count,
                            "activity_count": r.activity_count,
                            "last_activity_at": r.last_activity_at,
                            "state": r.state,
                            "pinned": r.pinned,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
                );
                return Ok(());
            }
            if rows.is_empty() {
                println!("curator: no skills found");
                return Ok(());
            }
            let agent = rows.iter().filter(|r| r.provenance == "agent").count();
            println!(
                "skills: {} total  (agent={}  user={})",
                rows.len(),
                agent,
                rows.len() - agent
            );
            println!();
            println!(
                "  {:<40}  {:<6}  {:>4}  {:>4}  {:>5}  {:>4}  last_activity",
                "skill", "origin", "use", "view", "patch", "act"
            );
            for row in &rows {
                let name: String = row.name.chars().take(40).collect();
                println!(
                    "  {:<40}  {:<6}  {:>4}  {:>4}  {:>5}  {:>4}  {}",
                    name,
                    row.provenance,
                    row.use_count,
                    row.view_count,
                    row.patch_count,
                    row.activity_count,
                    curator::fmt_ts(row.last_activity_at.as_deref())
                );
            }
            Ok(())
        }
        CuratorAction::Prune { days, dry_run, yes } => {
            if days < 1 {
                return Err(format!("--days must be >= 1 (got {})", days));
            }
            let candidates = curator::prune_candidates(&home, days);
            if candidates.is_empty() {
                println!(
                    "curator: nothing to prune (no unpinned agent-created skills idle >= {}d)",
                    days
                );
                return Ok(());
            }
            println!("curator: {} skill(s) idle >= {}d:", candidates.len(), days);
            for (name, idle) in &candidates {
                println!("  {:<40} idle {}d", name, idle);
            }
            if dry_run {
                println!("\n(dry run — no changes made)");
                return Ok(());
            }
            if !yes {
                print!("\nArchive {} skill(s)? [y/N] ", candidates.len());
                std::io::Write::flush(&mut std::io::stdout()).ok();
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("curator: aborted");
                    return Err("aborted".into());
                }
            }
            let mut archived = 0usize;
            let mut failures: Vec<(String, String)> = Vec::new();
            for (name, _) in &candidates {
                let (ok, message) = skill_usage::archive_skill(&home, name);
                if ok {
                    archived += 1;
                } else {
                    failures.push((name.clone(), message));
                }
            }
            println!("\ncurator: archived {}/{}", archived, candidates.len());
            if !failures.is_empty() {
                println!("failures:");
                for (name, message) in &failures {
                    println!("  {}: {}", name, message);
                }
                return Err("some archives failed".into());
            }
            Ok(())
        }
        CuratorAction::Adopt { skill, all_unmanaged, dry_run, yes } => {
            let mut names = skill;
            if all_unmanaged {
                if !names.is_empty() {
                    return Err("pass either skill names or --all-unmanaged, not both".into());
                }
                names = skill_usage::list_unmanaged_skill_names(&home);
                if names.is_empty() {
                    println!("curator: no unmanaged skills to adopt");
                    return Ok(());
                }
            }
            if names.is_empty() {
                return Err("name a skill to adopt, or pass --all-unmanaged".into());
            }
            if dry_run {
                println!("curator: would adopt {} skill(s) (dry run):", names.len());
                for name in &names {
                    println!("  + {}", name);
                }
                return Ok(());
            }
            if all_unmanaged && !yes {
                println!(
                    "curator: adopt {} unmanaged skill(s) into curator management?",
                    names.len()
                );
                println!("  they become eligible for pruning");
                print!("  proceed? [y/N] ");
                std::io::Write::flush(&mut std::io::stdout()).ok();
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("curator: aborted");
                    return Err("aborted".into());
                }
            }
            let mut failed = 0usize;
            for name in &names {
                let (ok, message) = skill_usage::adopt_skill(&home, name);
                println!("curator: {}", message);
                if !ok {
                    failed += 1;
                }
            }
            if names.len() > 1 {
                println!("curator: adopted {}/{}", names.len() - failed, names.len());
            }
            if failed > 0 { Err("some adoptions failed".into()) } else { Ok(()) }
        }
        CuratorAction::ListUnmanaged => {
            let rows = skill_usage::unmanaged_report(&home);
            if rows.is_empty() {
                println!("curator: no unmanaged skills — every skill has provenance");
                return Ok(());
            }
            println!("unmanaged skills ({}):", rows.len());
            let mut sorted = rows.clone();
            sorted.sort_by(|a, b| {
                a.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
            });
            for row in &sorted {
                let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let why = if row
                    .get("has_provenance_key")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    "created_by:null"
                } else {
                    "no marker"
                };
                let activity = row.get("activity_count").and_then(|v| v.as_u64()).unwrap_or(0);
                let last = row
                    .get("last_activity_at")
                    .and_then(|v| v.as_str())
                    .map(|ts| curator::fmt_ts(Some(ts)))
                    .unwrap_or_else(|| "never".to_string());
                println!(
                    "  {:<44} activity={:>4}  last_activity={:<14}  ({})",
                    name, activity, last, why
                );
            }
            println!(
                "\nadopt one with `ulnclaw curator adopt <name>`, or all with `ulnclaw curator adopt --all-unmanaged`"
            );
            Ok(())
        }
    }
}

// ── journey: the learning timeline (hermes hermes_cli/journey.py) ─────────

fn journey_cmd(
    action: Option<JourneyAction>,
    reveal: f64,
    play: bool,
    fps: u32,
    width: Option<usize>,
    height: Option<usize>,
    no_color: bool,
    json_flag: bool,
) -> Result<(), String> {
    use ulnclaw::learning_graph_render as render;

    let home = ulnclaw::config::ensure_home().map_err(|e| e.to_string())?;

    match action {
        Some(JourneyAction::List { no_color }) => {
            let payload = ulnclaw::learning_graph::build_learning_graph(&home);
            let mut nodes: Vec<serde_json::Value> = payload
                .get("nodes")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if nodes.is_empty() {
                println!("No learning yet.");
                return Ok(());
            }
            nodes.sort_by_key(|n| {
                n.get("timestamp")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            });
            let color = !no_color && std::io::stdout().is_terminal();
            for node in &nodes {
                let glyph = if node.get("kind").and_then(|v| v.as_str()) == Some("memory") {
                    render::MEMORY_GLYPH
                } else {
                    render::SKILL_GLYPH
                };
                let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("");
                let date = render::format_date(node.get("timestamp").and_then(|v| v.as_f64()));
                if color {
                    println!(
                        "\u{1b}[38;2;138;138;138m{}\u{1b}[0m  {} {}  \u{1b}[38;2;138;138;138m{}\u{1b}[0m",
                        id, glyph, label, date
                    );
                } else {
                    println!("{}  {} {}  {}", id, glyph, label, date);
                }
            }
            Ok(())
        }
        Some(JourneyAction::Delete { node, yes }) => {
            let detail = ulnclaw::learning_mutations::node_detail(&home, &node);
            if detail.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                println!(
                    "  {}",
                    detail
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("not found")
                );
                return Err("node not found".into());
            }
            if !yes {
                let label = detail.get("label").and_then(|v| v.as_str()).unwrap_or(&node);
                print!("  Delete '{}'? [y/N] ", label);
                std::io::Write::flush(&mut std::io::stdout()).ok();
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("  aborted");
                    return Err("aborted".into());
                }
            }
            let result = ulnclaw::learning_mutations::delete_node(&home, &node);
            println!(
                "  {}",
                result.get("message").and_then(|v| v.as_str()).unwrap_or("")
            );
            if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                Ok(())
            } else {
                Err("delete failed".into())
            }
        }
        Some(JourneyAction::Edit { node }) => {
            let detail = ulnclaw::learning_mutations::node_detail(&home, &node);
            if detail.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                println!(
                    "  {}",
                    detail
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("not found")
                );
                return Err("node not found".into());
            }
            let kind = detail.get("kind").and_then(|v| v.as_str()).unwrap_or("skill");
            let content = detail
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let suffix = if kind == "skill" { ".md" } else { ".txt" };
            let Some(edited) = open_in_editor(&content, suffix)? else {
                println!("  no changes");
                return Ok(());
            };
            if edited.trim() == content.trim() {
                println!("  no changes");
                return Ok(());
            }
            let result = ulnclaw::learning_mutations::edit_node(&home, &node, &edited);
            println!(
                "  {}",
                result.get("message").and_then(|v| v.as_str()).unwrap_or("")
            );
            if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                Ok(())
            } else {
                Err("edit failed".into())
            }
        }
        None => {
            let payload = ulnclaw::learning_graph::build_learning_graph(&home);
            if json_flag {
                println!("{}", serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?);
                return Ok(());
            }
            let color = !no_color && std::io::stdout().is_terminal();
            let (cols, rows) = term_size(width, height);
            let nodes = payload.get("nodes").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            if nodes == 0 {
                println!(
                    "No learning yet — use ulnclaw a while and your learned skills and memories will start mapping out here."
                );
                return Ok(());
            }
            if play {
                journey_play(&payload, cols, rows, color, fps)
            } else {
                let reveal = reveal.clamp(0.0, 1.0);
                print!("{}", journey_frame_text(&payload, cols, rows, reveal, color));
                Ok(())
            }
        }
    }
}

fn term_size(width: Option<usize>, height: Option<usize>) -> (usize, usize) {
    let env_cols = std::env::var("COLUMNS").ok().and_then(|v| v.parse().ok());
    let env_lines = std::env::var("LINES").ok().and_then(|v| v.parse().ok());
    let cols = width.or(env_cols).unwrap_or(90).max(40);
    let rows = height.or(env_lines).unwrap_or(30).max(10);
    (cols, rows)
}

fn open_in_editor(initial: &str, suffix: &str) -> Result<Option<String>, String> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(bin) = parts.next() else {
        return Err("no editor configured".into());
    };
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "ulnclaw-journey-{}-{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        suffix
    ));
    std::fs::write(&path, initial).map_err(|e| e.to_string())?;
    let status = std::process::Command::new(bin)
        .args(parts)
        .arg(&path)
        .status();
    let result = match status {
        Ok(status) if status.success() => {
            std::fs::read_to_string(&path).map(Some).map_err(|e| e.to_string())
        }
        Ok(_) => Err("editor exited non-zero".to_string()),
        Err(e) => Err(format!("editor failed: {}", e)),
    };
    std::fs::remove_file(&path).ok();
    result
}

/// Resolve a style run to a concrete 24-bit foreground color.
fn run_color(
    run: &ulnclaw::learning_graph_render::Run,
    palette: &std::collections::HashMap<String, String>,
) -> Option<(u8, u8, u8)> {
    use ulnclaw::learning_graph_render as render;
    let base = run.hex.clone().or_else(|| palette.get(&run.style).cloned())?;
    let faded = render::fade(palette, Some(&base), run.alpha)?;
    Some(render::hex_to_rgb(&faded))
}

fn row_to_text(
    row: &[ulnclaw::learning_graph_render::Run],
    palette: &std::collections::HashMap<String, String>,
    color: bool,
) -> String {
    let mut out = String::new();
    for run in row {
        if !color {
            out.push_str(&run.text);
            continue;
        }
        match run_color(run, palette) {
            Some((r, g, b)) => {
                out.push_str(&format!("\u{1b}[38;2;{};{};{}m{}\u{1b}[0m", r, g, b, run.text));
            }
            None => out.push_str(&run.text),
        }
    }
    out
}

fn journey_frame_text(
    payload: &serde_json::Value,
    cols: usize,
    rows: usize,
    reveal: f64,
    color: bool,
) -> String {
    use ulnclaw::learning_graph_render as render;

    let palette = render::derive_palette("#FFD700", true);
    let legend = render::build_legend(payload);
    let categories = render::category_legend(payload, 4);
    let summary = render::build_summary(payload);
    let axis = render::axis_labels(payload);
    // Lines are pad-left(2), so content must fit in cols-2.
    let inner = cols.saturating_sub(2).max(24);
    // Reserve rows for title/legend/blank/axis/footer/labels + summary.
    let field_rows = rows.saturating_sub(10 + summary.len()).max(6);
    let frame = render::render_graph(payload, inner, field_rows, reveal);
    let count = payload
        .get("nodes")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let mut parts: Vec<String> = Vec::new();

    if color {
        parts.push(format!(
            "\u{1b}[1;38;2;232;196;99m✦ Journey \u{1b}[0m\u{1b}[38;2;158;158;158m· learned skills & memories over time\u{1b}[0m"
        ));
    } else {
        parts.push("✦ Journey · learned skills & memories over time".to_string());
    }

    let mut legend_line = String::from("  ");
    for (i, item) in legend.iter().enumerate() {
        if i > 0 {
            legend_line.push_str("   ");
        }
        let glyph = item.get("glyph").and_then(|v| v.as_str()).unwrap_or("");
        let label = item.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let style = item.get("style").and_then(|v| v.as_str()).unwrap_or(render::STYLE_DIM);
        if color {
            let fake = render::Run {
                text: format!("{} ", glyph),
                style: style.to_string(),
                alpha: 1.0,
                hex: None,
            };
            legend_line.push_str(&row_to_text(&[fake], &palette, true));
            legend_line.push_str(&format!("\u{1b}[38;2;158;158;158m{}\u{1b}[0m", label));
        } else {
            legend_line.push_str(&format!("{} {}", glyph, label));
        }
    }
    parts.push(legend_line);

    if !categories.is_empty() {
        let mut cat_line = String::from("  ");
        for (i, item) in categories.iter().enumerate() {
            if i > 0 {
                cat_line.push_str("  ");
            }
            let glyph = item.get("glyph").and_then(|v| v.as_str()).unwrap_or("");
            let label = item.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let hex = item.get("color").and_then(|v| v.as_str()).unwrap_or("");
            if color && !hex.is_empty() {
                let (r, g, b) = render::hex_to_rgb(hex);
                cat_line.push_str(&format!("\u{1b}[38;2;{};{};{}m{} \u{1b}[0m", r, g, b, glyph));
                cat_line.push_str(&format!("\u{1b}[38;2;138;138;138m{}\u{1b}[0m", label));
            } else {
                cat_line.push_str(&format!("{} {}", glyph, label));
            }
        }
        parts.push(cat_line);
    }

    parts.push(String::new());

    for row in &frame.grid {
        if row.is_empty() {
            parts.push(String::new());
        } else {
            parts.push(format!("  {}", row_to_text(row, &palette, color)));
        }
    }

    // Date axis under the field (oldest → now).
    let (start, end) = axis;
    let gap = inner.saturating_sub(start.chars().count()).saturating_sub(end.chars().count()).max(1);
    if color {
        parts.push(format!(
            "  \u{1b}[38;2;138;138;138m{}\u{1b}[0m{}\u{1b}[38;2;138;138;138m{}\u{1b}[0m",
            start,
            " ".repeat(gap),
            end
        ));
    } else {
        parts.push(format!("  {}{}{}", start, " ".repeat(gap), end));
    }

    let pct = (reveal * 100.0).round() as i64;
    if color {
        parts.push(format!(
            "  \u{1b}[38;2;138;138;138m◷ \u{1b}[0m\u{1b}[38;2;232;196;99m{}\u{1b}[0m   \u{1b}[38;2;138;138;138m{}/{} revealed · {}%\u{1b}[0m",
            if frame.date.is_empty() { "—" } else { &frame.date },
            frame.visible,
            count,
            pct
        ));
    } else {
        parts.push(format!(
            "  ◷ {}   {}/{} revealed · {}%",
            if frame.date.is_empty() { "—" } else { &frame.date },
            frame.visible,
            count,
            pct
        ));
    }

    if !frame.labels.is_empty() {
        parts.push(String::new());
        if color {
            parts.push("  \u{1b}[38;2;158;158;158mcharted signals\u{1b}[0m".to_string());
        } else {
            parts.push("  charted signals".to_string());
        }
        for item in frame.labels.iter().take(6) {
            let key = item.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let glyph = item.get("glyph").and_then(|v| v.as_str()).unwrap_or("");
            let label = item.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let meta = item.get("meta").and_then(|v| v.as_str()).unwrap_or("");
            let style = item.get("style").and_then(|v| v.as_str()).unwrap_or(render::STYLE_DIM);
            let alpha = item.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let meta: String = if meta.chars().count() <= 32 {
                meta.to_string()
            } else {
                meta.chars().take(29).collect::<String>() + "…"
            };
            if color {
                let fake = render::Run {
                    text: format!("{} {}", glyph, label),
                    style: style.to_string(),
                    alpha,
                    hex: None,
                };
                parts.push(format!(
                    "  \u{1b}[38;2;178;178;178m{} \u{1b}[0m{}  \u{1b}[38;2;138;138;138m{}\u{1b}[0m",
                    key,
                    row_to_text(&[fake], &palette, true),
                    meta
                ));
            } else {
                parts.push(format!("  {} {} {}  {}", key, glyph, label, meta));
            }
        }
    }

    for line in &summary {
        if color {
            parts.push(format!("  \u{1b}[38;2;158;158;158m{}\u{1b}[0m", line));
        } else {
            parts.push(format!("  {}", line));
        }
    }

    parts.join("\n") + "\n"
}

fn journey_play(
    payload: &serde_json::Value,
    cols: usize,
    rows: usize,
    color: bool,
    fps: u32,
) -> Result<(), String> {
    let frames = 42usize;
    let delay = std::time::Duration::from_secs_f64(1.0 / fps.clamp(1, 60) as f64);
    let mut out = std::io::stdout();
    if color {
        // Clear the screen once, then home the cursor per frame.
        write!(out, "\u{1b}[2J").ok();
    }
    for i in 0..frames {
        let reveal = i as f64 / (frames - 1) as f64;
        let text = journey_frame_text(payload, cols, rows, reveal, color);
        if color {
            write!(out, "\u{1b}[H{}", text).ok();
        } else {
            write!(out, "{}", text).ok();
        }
        out.flush().ok();
        std::thread::sleep(delay);
    }
    let text = journey_frame_text(payload, cols, rows, 1.0, color);
    if color {
        write!(out, "\u{1b}[H{}", text).ok();
    } else {
        write!(out, "{}", text).ok();
    }
    out.flush().ok();
    Ok(())
}

fn models_cmd(action: ModelsAction) -> Result<(), String> {
    use ulnclaw::models_dev as md;
    match action {
        ModelsAction::Providers => {
            let providers = md::list_providers(true);
            if providers.is_empty() {
                return Err(
                    "models.dev catalog unavailable (network offline and no local cache)"
                        .to_string(),
                );
            }
            println!(
                "{:<24} {:<36} {:>7}  env",
                "id", "name", "models"
            );
            for provider in providers {
                println!(
                    "{:<24} {:<36} {:>7}  {}",
                    provider.id,
                    provider.name,
                    provider.model_count,
                    provider.env.join(",")
                );
            }
            let cache = md::cache_info();
            println!(
                "\ncatalog: {} providers (age {}s, fresh={})",
                cache.providers,
                cache.age_secs.round() as u64,
                cache.fresh
            );
        }
        ModelsAction::List { provider, all, refresh } => {
            if refresh {
                md::fetch_models_dev_opts(true, true);
            }
            let models = if all {
                md::list_provider_models(&provider)
            } else {
                md::list_agentic_models(&provider)
            };
            if models.is_empty() {
                return Err(format!(
                    "no models found for provider '{provider}' (not in the models.dev catalog?)"
                ));
            }
            for model in models {
                println!("{model}");
            }
        }
        ModelsAction::Info { provider, model } => {
            let Some(info) = md::get_model_info(&provider, &model) else {
                return Err(format!("model '{model}' not found for provider '{provider}'"));
            };
            println!("{} ({})", info.name, info.id);
            println!("  provider:    {}", info.provider_id);
            if !info.family.is_empty() {
                println!("  family:      {}", info.family);
            }
            println!(
                "  limits:      context={} output={}{}",
                info.context_window,
                info.max_output,
                info.max_input
                    .map(|v| format!(" input={v}"))
                    .unwrap_or_default()
            );
            println!("  cost:        {}", info.format_cost());
            println!("  capabilities: {}", info.format_capabilities());
            if !info.input_modalities.is_empty() {
                println!("  modalities:  in={} out={}", info.input_modalities.join("+"), info.output_modalities.join("+"));
            }
            if !info.knowledge_cutoff.is_empty() {
                println!("  knowledge:   {}", info.knowledge_cutoff);
            }
            if !info.status.is_empty() {
                println!("  status:      {}", info.status);
            }
        }
        ModelsAction::Refresh => {
            md::fetch_models_dev_opts(true, true);
            let cache = md::cache_info();
            if cache.providers == 0 {
                return Err("models.dev refresh failed (see debug log); no cache available".to_string());
            }
            println!(
                "models.dev cache refreshed: {} providers (fresh={})",
                cache.providers, cache.fresh
            );
        }
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
        SkillAction::Scan { name, source, json, force } => {
            let Some(skill_dir) = ulnclaw::skills::guard::find_skill_dir(&dir, &name) else {
                return Err(format!("skill '{}' not found in {}", name, dir.display()));
            };
            let result = ulnclaw::skills::guard::scan_skill(&skill_dir, &source);
            if json {
                let mut value = serde_json::to_value(&result).map_err(|e| e.to_string())?;
                let (allowed, reason) = ulnclaw::skills::guard::should_allow_install(&result, force);
                value["decision"] = serde_json::json!({
                    "allowed": allowed,
                    "reason": reason,
                    "scanner": ulnclaw::skills::guard::SCANNER_VERSION,
                });
                println!("{}", serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?);
            } else {
                println!("{}", ulnclaw::skills::guard::format_scan_report(&result));
            }
            let (allowed, _) = ulnclaw::skills::guard::should_allow_install(&result, force);
            if allowed == Some(false) {
                return Err("scan blocked the skill".to_string());
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

async fn cron_cmd(config: &UlncLawConfig, action: CronAction) -> Result<(), String> {
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
        CronAction::Run { id } => {
            let Some(mut job) = store.get(&id).map_err(|e| e.to_string())? else {
                return Err(format!("no cron job named '{}' found", id));
            };
            if job.prompt.trim().is_empty() {
                return Err("job has no prompt to run".into());
            }
            // Unattended execution: the agent runs inside the cron
            // approval scope (`approvals.cron_mode` applies).
            use ulnclaw::tools::context::CronRunner;
            let agent = make_agent(config, false, None).await?;
            let started = std::time::Instant::now();
            match agent.run_prompt(&job.prompt, &job.skills).await {
                Ok(answer) => {
                    job.last_run = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs_f64())
                            .unwrap_or(0.0),
                    );
                    job.last_status = Some(format!(
                        "ok (manual run, {}s)",
                        started.elapsed().as_secs()
                    ));
                    store.update(&job).map_err(|e| e.to_string())?;
                    println!("{}", answer);
                }
                Err(e) => {
                    job.last_status = Some(format!("error: {}", e));
                    store.update(&job).ok();
                    return Err(e.to_string());
                }
            }
        }
    }
    Ok(())
}
