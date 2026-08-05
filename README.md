# ulnclaw 🦞

[English](#english) | [中文](#中文)

---

## English

**A high-performance AI agent engine written in Rust — a port of [hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3)**

ulnclaw re-implements the Hermes Agent engine in Rust: the same tool surface
(50+ built-in tools), the same SQLite session/memory/skills/cron storage
layout, the same toolset composition — with native performance and a single
static musl binary. See the [parity matrix](docs/en/hermes-parity.md) for the
full feature-by-feature mapping — core parity with hermes-agent v2026.8.3 is
complete, including the messaging-platform gateways (Telegram/Discord/Slack),
the plugin + shell-hook system, secrets vaults, computer-use, OAuth login +
skill sync, and a Tauri desktop GUI (`desktop/`).

### Key Features

- **🤖 Agent loop** — tool calling with iteration budgets, usage accounting, memory injection, step/tool callbacks
- **🔧 50+ built-in tools** — terminal/process, file read/write/patch/search, web search/extract, X (Twitter) search via xAI (`x_search`, opt-in toolset + `XAI_API_KEY`), video understanding (`video_analyze`, opt-in `video` toolset), memory, todo, session search, clarify, skills, delegation, execute_code, cronjob, vision, image generation, video generation (`video_generate` registry — BFL FLUX 3 via the Nous tool gateway, xAI Imagine incl. edit/extend, FAL six-family queue, DeepInfra), desktop projects (`project_list/create/switch`, opt-in `project` toolset), learning timeline (`ulnclaw journey`), TTS, Home Assistant, kanban, tool search
- **🧰 Toolsets** — hermes-compatible grouping (`coding`, `web`, `file`, `safe`, `debugging`, ...) with composition and enable/disable policy
- **🛡️ Approval system** — command normalization, hardline floor (auto-block), confirm-before-run for costly operations; REPL prompts plus gateway run approvals over HTTP with fail-closed timeout and persisted `always` grants
- **💾 SQLite state** — sessions/messages with FTS5 full-text search, lineage (parent/child sessions), cron jobs, kanban board; offline non-destructive `sessions recover` for damaged databases (rowid salvage, orphan-session reconstruction, FTS rebuild)
- **🗜️ Context compression** — budget-triggered middle-turn summarization via a secondary model call
- **🤝 Delegation** — parallel sub-agents with isolated contexts and depth limits; top-level delegations run fire-and-forget in the background (live transcripts under `cache/delegation/live/`) and one consolidated result re-enters the conversation when the batch finishes; dispatches and results persist in the SQLite delegation registry, so finished work survives restarts (rows still running after a crash recover as terminal "outcome unknown" reports)
- **🧬 Mixture of Agents** — `[moa]` presets fan a prompt out to reference models in parallel and synthesize their answers via an aggregator (`ulnclaw moa run/list/delete`, REPL `/moa`)
- **🗺️ Model catalog** — models.dev-backed multi-provider inventory with a three-tier cache (memory → disk → network with 5-min failure backoff): `ulnclaw models providers|list|info|refresh`, gateway `/api/model/options` catalog enrichment + `?refresh=true`, `ULNCLAW_MODELS_DEV_URL` mirror override
- **⏰ Cron** — `30m` / `every 2h` / `0 9 * * *` / ISO one-shot schedules with a poll scheduler
- **📐 Blueprints** — skills with a `metadata.hermes.blueprint.schedule` frontmatter become cron jobs (`skills blueprints`, `skills schedule/unschedule`)
- **🛡️ Skills guard** — `skills scan <name>` runs the `skills-guard-v1` static scanner (119 threat patterns, invisible-unicode + structural checks, source trust levels) before you install or run third-party skills; dangerous skills are blocked even from trusted sources
- **🔌 MCP client** — stdio JSON-RPC: any MCP server's tools appear as `mcp__<server>__<tool>`; npx/uvx launches get an OSV malware preflight (MAL-* advisories block, fail-open)
- **🌍 Browser automation** — CDP WebSocket client with a built-in supervisor (auto-launches headless Chrome/Chromium, or point `ULNCLAW_BROWSER_CDP` at your own): accessibility snapshots with element refs, click/type/scroll/press/screenshot/JS evaluate/dialogs
- **🌐 Browser automation** — 12 `browser_*` tools over a CDP WebSocket client (accessibility snapshots with element refs, click/type/scroll/press, screenshots + vision, console/eval, raw CDP, dialogs); managed headless Chrome (`ULNCLAW_BROWSER_CDP=auto`), any existing DevTools endpoint, or the Camofox anti-detect REST backend (`CAMOFOX_URL`), with hermes-grade SSRF guards (metadata floor, private-address gating, redirect rechecks, raw-CDP allowlist) and forced secret redaction on browser output
- **🚪 HTTP gateway** — `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` + `/v1/responses` (with session continuity), `stream: true` SSE streaming on both (token deltas, tool-progress/function-call events), async `/v1/runs` with SSE events + approval resolution, sessions API (incl. `PATCH`/fork + enforced per-session model lock), `/api/jobs` cron management (CRUD + pause/resume/run) with a built-in scheduler that auto-runs due jobs, `/v1/skills` + `/v1/toolsets` discovery, `/api/model/options` (models.dev catalog enrichment), Prometheus `/metrics`, token accounting `/api/usage`, background-delegation registry `/v1/delegations`, live browser CDP control `/v1/browser/status|connect|disconnect`, bearer auth
- **🖥️ Terminal environments** — run `terminal` locally (default), in docker (auto container creation), or over ssh (`[terminal] backend`)
- **🩺 Terminal failure intelligence** — failed commands carry actionable guidance: benign exit codes are explained (`grep=1` → "no matches, not an error" as `exit_code_meaning`) and well-known failure shapes get one recovery `hint` (command/module not found, git conflicts, gh field drift & rate limits, permission errors, exits 124/126/137)
- **🔬 Environment probe** — when the terminal backend is local, one deterministic Python-toolchain line (pip/python3 mismatch, missing pip module, PEP 668, missing bare `python`) is injected into the system prompt — silent on healthy machines, background-probed with fail-open timeout (`[agent] environment_probe`)
- **🖥️ Desktop bridge tools** — `close_terminal` / `read_terminal` / `focus_pane` / `open_preview` / `react_to_message` (emoji tapbacks, `[display] message_reactions`) for GUI hosts: gated on `ULNCLAW_DESKTOP=1`, routed through the `desktop` bridge (host-installed emitter receives `terminal.close` / `pane.reveal` / `preview.open` events per UI session); never kill processes, report "desktop only" without a wired host
- **🧹 ANSI stripping** — terminal/execute_code output is cleaned of ECMA-48 escape sequences (colors, cursor moves, OSC titles, 8-bit C1) before it reaches the model, so escapes never leak into context or file writes
- **🔒 Sandbox credential scrub** — terminal/execute_code child processes run with provider & tool credentials stripped from their environment (hermes GHSA-rhgp-j443-p4rf semantics); skills' `required_environment_variables` and `[terminal] env_passthrough` allowlist the rest — provider credentials can never be allowlisted
- **🛡️ Web 工具 SSRF 防护** —— `web_extract` 拦截私有/内网目标（环回、RFC1918、CGNAT、ULA、IPv4 映射 IPv6），并拒绝在 URL 中嵌入凭证；云元数据端点（169.254.169.254 等）永远拦截，重定向逐跳重验；可用 `[security] allow_private_urls` 或 `ULNCLAW_ALLOW_PRIVATE_URLS=true` 放开
- **🛡️ SSRF guard for web tools** — `web_extract` blocks private/internal targets (loopback, RFC1918, CGNAT, ULA, IPv4-mapped IPv6) and refuses URLs embedding credentials; cloud metadata endpoints (169.254.169.254 etc.) are always blocked, redirects are re-validated hop by hop; opt out with `[security] allow_private_urls` or `ULNCLAW_ALLOW_PRIVATE_URLS=true`
- **🚫 Binary guard** — `read_file` refuses ~80 binary extensions (images, archives, executables, fonts, bytecode, databases) with a pointer to vision_analyze/terminal; `.pdf` stays readable
- **📏 Configurable output limits** — `[tool_output] max_bytes/max_lines/max_line_length` tune terminal truncation, read_file pagination, and per-line clamping without patching source
- **🕵️ Secret redaction** — ~55 vendor key prefixes, JWTs, private keys, DB connstrings, auth headers, and env-dump `KEY=value` pairs are masked before output reaches the model; file content gets non-reusable sentinels so truncated keys are never written back
- **📸 Checkpoints** — transparent git-backed snapshots before file edits (shared shadow store, per-project chains), `ulnclaw checkpoints list/restore/diff/prune`
- **📝 Working diff** — `ulnclaw diff [--staged|--all]` shows what changed in a git worktree (untracked files included), REPL `/gitdiff`
- **🌐 Providers** — OpenAI-compatible endpoints (OpenAI, OpenRouter, DashScope, Ollama, llama.cpp) plus a native Anthropic Messages API provider (tool_use/tool_result blocks, SSE streaming, OAuth bearer), keyless local providers; per-task auxiliary routing (`[auxiliary.compression]`, `[auxiliary.vision]`, `[auxiliary.title_generation]`) sends secondary calls to a different provider/model; `[model] fallbacks` failover chain with per-turn primary restore
- **📡 Messaging platforms** — Telegram/Discord/Slack/Signal adapters run inside `ulnclaw gateway` (`[messaging.*]`, Signal via a signal-cli HTTP daemon), WhatsApp Cloud + Microsoft Graph ingress mount as gateway webhook routes (`/webhooks/whatsapp` HMAC-verified, `/webhooks/msgraph` clientState-verified), plus a generic signed-webhook platform (`[messaging.webhook]` routes at `/webhooks/hook/<name>` — Svix/GitHub/GitLab/HMAC-V2 signature schemes, per-route rate limits, delivery-id idempotency, `deliver_only` zero-LLM push), and BlueBubbles iMessage (`[messaging.bluebubbles]` → `/webhooks/bluebubbles` password-authenticated webhook, LRU-cached chat-GUID resolution, REST text/attachment sends), and Weixin personal accounts (`[messaging.weixin]` — Tencent iLink Bot API long-polling, QR login via `ulnclaw weixin login`, AES-128-ECB CDN media both ways, context_token echo sends), and QQ (`[messaging.qq]` — official QQ Bot API v2 WebSocket gateway + REST, markdown replies, chunked media uploads, `asr_refer_text` voice transcripts): allowlist-gated pairing (fail closed) plus hermes-style interactive pairing codes (`pairing list/approve/revoke/clear-pending`), media attachments cached under `media-cache/` and delivered as path references (outbound `MEDIA:` tags upload natively on Telegram/Discord/Slack; WhatsApp media rides the Graph `/media` endpoint both ways), one persistent session per chat, hermes-style reply chunking. The `clarify` tool works in chats: WhatsApp renders native buttons/list sheets, other platforms numbered text; button taps and follow-up text resolve the pending question
- **🎙️ Voice-note transcription (STT)** — inbound audio/voice messages are transcribed before the agent turn (`[stt]` config): built-in `local_command` / `groq` / `openai` / `mistral` / `xai` / `elevenlabs` / `deepinfra` providers plus custom `[stt.providers.<name>]` command providers, transcripts echoed back as 🎙️ messages and injected into the turn with hermes fallback/sentinel semantics; a `transcribe_audio` tool (opt-in `stt` toolset) covers arbitrary files. The Python-only faster-whisper `local` provider is replaced by `stt.local.command` / cloud backends
- **📋 Kanban engine** — `ulnclaw kanban`: multi-board task engine in `kanban.db` (hermes statuses todo/ready/running/scheduled/blocked/done/archived with icons), TTL claim locks with stale takeover + heartbeats, comments + event trail, board CRUD; lifecycle transitions fire the `kanban_task_*` plugin hooks
- **🔌 Plugins & hooks** — directory plugins (`~/.ulnclaw/plugins/<name>/plugin.toml`: hooks + subprocess tools) and `[hooks]` config shell hooks with hermes first-use consent (`plugins list/enable/disable/accept-hooks`, `hooks list/test/revoke/doctor`); the core fires all 13 hook events hermes emits at runtime (pre/post tool & LLM calls, API request lifecycle, session boundaries, gateway dispatch gating)
- **🔑 Secrets vaults** — external secret sources applied at startup before providers read env (`secrets status/sync`): command helper, Bitwarden Secrets Manager (`bws` — pinned auto-install, AES-GCM-encrypted TTL cache, `secrets bitwarden setup` wizard), 1Password (`op://` refs, `secrets onepassword setup/set`) with full hermes precedence semantics
- **🖱️ Computer use** — `computer_use` tool via the cua-driver daemon (MCP over stdio, full hermes schema), approval-gated like hermes; `computer-use status/doctor/install`
- **🔄 OAuth + skill sync** — `auth login` RFC 8628 device flow against any `[oauth]` provider; `sync status/pull/push/now` keeps skills in sync over HTTP(S) or a shared directory
- **🖥️ Desktop GUI** — `desktop/`: a Tauri 2 shell (replacing hermes' Electron app) that hosts the chat UI (session rename/delete hover actions, live tool-progress strip from `hermes.tool.progress` SSE events, `/`-slash completion popup over gateway commands + installed skills, expandable tool-call cards from `hermes.tool.started/completed` SSE events, clipboard-image paste uploaded via `POST /api/uploads` and attached as media path references) and manages the gateway child process; the gateway's local-app CORS also serves any browser dashboard

### CLI Quick Start

```bash
cargo build --release --target x86_64-unknown-linux-musl

# Write a default config to ~/.ulnclaw/config.toml
./ulnclaw init

# One-shot run
./ulnclaw run "Summarize the README.md file"

# Interactive chat (slash commands: /new /search /memory /skills /sessions /rollback /diff /recap /goal /subgoal /focus /verbose /stash /paste ...)
./ulnclaw chat
./ulnclaw chat --resume <session-id>   # resume a session by id or unique prefix (-r)
./ulnclaw chat --continue              # continue the most recent session (-c)

# Management subcommands
./ulnclaw tools            # list toolsets and enabled tools
./ulnclaw sessions list    # recent sessions from state.db
./ulnclaw sessions search "auth refactor"
./ulnclaw sessions export <session-id> --out ./exports --format md|html
./ulnclaw sessions recover ./damaged-state.db   # offline db recovery
./ulnclaw sessions repair          # repair malformed state.db schema (--check-only)
./ulnclaw sessions browse          # interactive picker: filter + resume sessions
./ulnclaw sessions retitle-skills  # fix titles that leaked /skill scaffolds (--apply)
./ulnclaw sessions delete|rename|optimize # per-session delete / rename / FTS-merge + VACUUM
./ulnclaw skills list
./ulnclaw skills blueprints    # schedulable skills (skills schedule <name>)
./ulnclaw skills scan <name>   # security scan before trusting a skill (--json, --source, --force)
./ulnclaw cron list          # cron jobs (cron run <id> executes one immediately)
./ulnclaw suggestions        # suggested automations (accept/dismiss/catalog/clear)
./ulnclaw moa list           # MoA presets (run: ./ulnclaw moa run "<prompt>")
./ulnclaw models providers   # models.dev catalog (list/info/refresh)
./ulnclaw checkpoints list   # filesystem snapshots ([checkpoints] enabled = true)
./ulnclaw diff               # git working-tree diff (--staged / --all)
./ulnclaw doctor             # diagnose config/deps (--fix, --online, --json)
./ulnclaw insights           # usage analytics over sessions (--days, --source, --json)
./ulnclaw status             # status of all components (--deep)
./ulnclaw logs               # tail/filter logs (-f, -n, --level, --since, --component)
./ulnclaw update --check   # check for updates (ulnclaw update applies: stash -> ff pull -> rebuild)
./ulnclaw config           # show/get/set/unset config (env-style keys go to .env)
./ulnclaw secrets status   # external secret sources (secrets sync [--apply] fetches now)
./ulnclaw secrets bitwarden setup   # wizard: install bws, store token, pick project (also: install/status/token/disable; onepassword setup/status/set/remove/disable)
./ulnclaw computer-use status # background desktop control via cua-driver (doctor/install)
./ulnclaw plugins list      # plugins + shell hooks (enable/disable/accept-hooks)
./ulnclaw kanban list       # kanban task engine (init/boards/create/claim/done/block/comment/...)
./ulnclaw hooks doctor      # probe every consented hook (list/test/revoke)
./ulnclaw pairing list      # DM pairing codes for unknown senders (approve/revoke/clear-pending)
./ulnclaw weixin login      # WeChat iLink QR-scan login for [messaging.weixin]
./ulnclaw auth login        # OAuth device-flow login (status/refresh/logout/open)
./ulnclaw sync status       # skill sync (pull/push/now/enable/disable/device)
./ulnclaw completion bash  # shell completions (bash/zsh/fish/elvish/powershell)
./ulnclaw dump             # copy-pasteable setup summary for support (--show-keys)
./ulnclaw version          # version + install info + update status
./ulnclaw uninstall        # remove code/PATH entries/wrappers (--full wipes data, --dry-run, --yes)
./ulnclaw memory           # persistent memory status; `memory reset [all|memory|user]`
./ulnclaw approvals        # terminal approval mode; `approvals manual|smart|off` to set
./ulnclaw prompt-size      # system prompt + tool-schema footprint (--json)
./ulnclaw debug report     # redacted diagnostic bundle for support (--no-redact)
./ulnclaw bundles          # skill bundles: load N skills under one /command
./ulnclaw import-agent     # import Claude Code / Codex setups (--dry-run)
./ulnclaw security audit   # OSV.dev audit of pinned MCP packages (--json)
./ulnclaw fallback         # fallback chain (add/remove/clear provider:model entries)
./ulnclaw backup           # zip backup of home (-q quick snapshot, backup list/restore/prune)
./ulnclaw import b.zip     # restore a backup zip (runtime-state files skipped, secrets 0600)

# Browser automation: auto mode launches a managed headless Chrome/Chromium;
# or point browser_* tools at an existing browser with remote debugging
export ULNCLAW_BROWSER_CDP=http://127.0.0.1:9222     # or ws://.../devtools/browser/... or "auto"
# or route browser_* through a Camofox anti-detect browser server:
# export CAMOFOX_URL=http://127.0.0.1:9377            # + optional CAMOFOX_API_KEY

# HTTP gateway (OpenAI-compatible API server, default 127.0.0.1:8642)
./ulnclaw gateway --host 127.0.0.1 --port 8642
# messaging platforms run inside the gateway ([messaging.telegram|discord|slack])
# the Tauri desktop app (desktop/) and any browser dashboard talk to this
# gateway over HTTP/SSE (local-app CORS is built in; see desktop/README.md)
curl -H "Authorization: Bearer $ULNCLAW_GATEWAY_KEY" \\
     -H "Content-Type: application/json" \\
     -d '{"messages":[{"role":"user","content":"Hello!"}]}' \\
     http://127.0.0.1:8642/v1/chat/completions
```

Example `~/.ulnclaw/config.toml`:

```toml
# timezone = "Asia/Shanghai"   # IANA zone for prompt timestamps
                               # (ULNCLAW_TIMEZONE / HERMES_TIMEZONE env override)

[model]
provider = "ollama"            # or "openai", "anthropic", "dashscope", ...
model = "qwen3:32b"
base_url = "http://localhost:11434/v1"
# max_retries = 2              # retry 429/5xx/network with backoff
# fallbacks = ["openai:gpt-5.2-mini", "ollama:qwen3:32b"]  # failover chain

[agent]
max_iterations = 90
approval = true                # y/N prompt before dangerous commands
environment_probe = true       # one-line Python toolchain note in the system prompt

# [approvals]
# timeout = 300                # gateway approvals fail closed after N seconds
# mode = "manual"              # manual | smart (aux-LLM guardian) | off
# cron_mode = "deny"           # deny | approve — unattended cron runs
# smart_policy = ""            # operator rules for the smart-approval guardian

[delegation]
max_concurrent_children = 3

# [[mcp.servers]]
# name = "filesystem"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me"]

[gateway]
host = "127.0.0.1"
port = 8642
# key = "sk-..."        # bearer token; env ULNCLAW_GATEWAY_KEY overrides
# multiplex_profiles = false  # true = serve /p/<profile>/... mirrors, each
                              # backed by its [profiles.<name>] override

# [terminal]
# backend = "docker"    # "local" (default) | "docker" | "ssh"
# container = "ulnclaw-dev"
# image = "ubuntu:24.04"
# env_passthrough = ["TENOR_API_KEY"]   # vars allowed past the sandbox credential scrub

# [tool_output]
# max_bytes = 100000        # terminal output head+tail cap
# max_lines = 2000          # read_file pagination cap
# max_line_length = 2000    # per-line clamp ('... [truncated]')

# [security]
# allow_private_urls = false  # true lets web tools fetch private/internal IPs
#                             # (cloud metadata endpoints stay blocked either way;
#                             #  env ULNCLAW_ALLOW_PRIVATE_URLS also works)

# [checkpoints]
# enabled = true        # transparent snapshots before write_file/patch

# Auxiliary model routing: run secondary calls on a different model.
# [auxiliary.compression]   # context-compression summaries
# provider = "openai"
# model = "gpt-5.2-mini"
# [auxiliary.vision]        # image analysis (vision_analyze / browser_vision)
# model = "gpt-5.2"
# [auxiliary.title_generation]  # session titles after the first exchange
# enabled = true            # kill switch (is_truthy_value semantics)
# language = ""             # pin title language; blank matches the user

# Mixture of Agents: parallel reference fan-out + aggregator synthesis
# [moa]
# default_preset = "default"
# [[moa.presets.default.reference_models]]
# provider = "ollama"
# model = "qwen3:32b"
# [moa.presets.default.aggregator]
# provider = "openai"
# model = "gpt-5.2"
```

### Library Quick Start

```rust
use ulnclaw::prelude::*;
use ulnclaw::{register_builtin_tools, ToolRegistry, SqliteSessionStore};

#[tokio::main]
async fn main() -> Result<()> {
    let provider = OpenAiProvider::builder()
        .endpoint("http://localhost:11434/v1")
        .model("qwen3:32b")
        .build()?;

    let mut tools = ToolRegistry::new();
    register_builtin_tools(&mut tools);          // all 50+ hermes-style tools

    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig { approval: false, ..Default::default() })
        .with_store(Arc::new(SqliteSessionStore::open_default()?));

    println!("{}", agent.chat("List the files in this directory").await?);
    Ok(())
}
```

### Documentation

- [Hermes Parity Matrix](docs/en/hermes-parity.md) — tool/feature mapping vs hermes-agent v2026.8.3
- [Architecture Guide](docs/en/architecture.md) · [API Reference](docs/en/api-reference.md)
- [Integration Guide](docs/en/integration.md) · [Development Guide](docs/en/development.md)
- [Tool System](docs/en/tools.md) · [Provider System](docs/en/providers.md)

### Building & Testing

```bash
cargo test                     # 893 tests
cargo build --release --target x86_64-unknown-linux-musl   # static binary
```

### License

MIT OR Apache-2.0

---

## 中文

**Rust 编写的高性能 AI Agent 引擎 —— [hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3) 的 Rust 移植**

ulnclaw 用 Rust 重新实现了 Hermes Agent 引擎：相同的工具面（50+ 内置工具）、
相同的 SQLite 会话/记忆/技能/定时任务存储布局、相同的工具集组合方式 ——
原生性能，单一静态 musl 二进制。完整的逐项对标见
[对标矩阵](docs/zh/hermes-parity.md) —— 与 hermes-agent v2026.8.3 的核心
对齐已完成，含消息平台网关（Telegram/Discord/Slack）、插件与 shell 钩子
体系、Secrets 保险库、computer-use、OAuth 登录 + 技能同步，以及 Tauri
桌面 GUI（`desktop/`）。

### 核心特性

- **🤖 Agent 循环** —— 工具调用、迭代预算、用量统计、记忆注入、步骤/工具回调
- **🔧 50+ 内置工具** —— terminal/process、文件读/写/补丁/搜索、web 搜索/抽取、经 xAI 的 X（Twitter）搜索（`x_search`，可选工具集 + `XAI_API_KEY`）、视频理解（`video_analyze`，可选 `video` 工具集）、记忆、todo、会话搜索、clarify、技能、委派、execute_code、cronjob、视觉、图像生成、视频生成（`video_generate` 注册表 —— BFL FLUX 3 经 Nous 工具网关、xAI Imagine 含 edit/extend、FAL 六家族队列、DeepInfra）、桌面项目（`project_list/create/switch`，可选 `project` 工具集）、学习时间线（`ulnclaw journey`）、TTS、Home Assistant、kanban、工具搜索
- **🧰 工具集** —— hermes 兼容分组（`coding`、`web`、`file`、`safe`、`debugging`……），支持组合与启用/禁用策略
- **🛡️ 审批系统** —— 命令归一化、硬性底线（自动阻止）、高成本操作先确认再执行；REPL 提示 + 网关 HTTP 运行审批（fail-closed 超时、`always` 授权持久化）
- **💾 SQLite 状态库** —— 会话/消息 FTS5 全文检索、会话血缘（父子会话）、定时任务、kanban 看板；受损数据库的离线非破坏性 `sessions recover`（rowid 抢救、孤儿会话重建、FTS 重建）
- **🗜️ 上下文压缩** —— 预算触发，中段对话经二次模型调用摘要
- **🤝 委派** —— 并行子代理，隔离上下文，深度限制；顶层委派后台即发即忘（实时记录在 `cache/delegation/live/`），整批完成后以单条汇总结果重回会话；派发与结果持久化于 SQLite 委派登记表，完成的工作可跨重启保留（崩溃后仍在运行的委派以终态 "outcome unknown" 报告恢复投递）
- **🧬 混合智能体（MoA）** —— `[moa]` 预设将提示词并行扇出给参考模型，经聚合器综合（`ulnclaw moa run/list/delete`、REPL `/moa`）
- **🗺️ 模型目录** —— models.dev 多 provider 清单，三级缓存（内存 → 磁盘 → 网络失败退避 5 分钟）：`ulnclaw models providers|list|info|refresh`、网关 `/api/model/options` 目录增强 + `?refresh=true`、`ULNCLAW_MODELS_DEV_URL` 镜像覆盖
- **⏰ 定时任务** —— `30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划 + 轮询调度器
- **📐 蓝图（Blueprints）** —— frontmatter 声明 `metadata.hermes.blueprint.schedule` 的技能可排程（`skills blueprints`、`skills schedule/unschedule`）
- **🛡️ 技能守卫** —— `skills scan <name>` 运行 `skills-guard-v1` 静态扫描器（119 条威胁模式、不可见 Unicode 与结构检查、来源信任等级），在安装/运行第三方技能前把关；dangerous 技能即使来自受信任仓库也会被拦截
- **🔌 MCP 客户端** —— stdio JSON-RPC：任意 MCP 服务器的工具以 `mcp__<server>__<tool>` 注册；npx/uvx 启动前经 OSV 恶意软件检查（MAL-* 通告阻止、fail-open）
- **🌍 浏览器自动化** —— CDP WebSocket 客户端 + 内置监督器（自动启动无头 Chrome/Chromium，或用 `ULNCLAW_BROWSER_CDP` 指向自有浏览器）：带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框
- **🌐 浏览器自动化** —— 12 个 `browser_*` 工具，基于 CDP WebSocket 客户端（带元素引用的可访问性快照、点击/输入/滚动/按键、截图 + 视觉、console/eval、原始 CDP、对话框）；托管无头 Chrome（`ULNCLAW_BROWSER_CDP=auto`）、任意已有 DevTools 端点，或 Camofox 反检测 REST 后端（`CAMOFOX_URL`），内置 hermes 级 SSRF 防护（元数据底线、私网地址门控、重定向复检、原始 CDP 白名单）并对浏览器输出强制脱敏
- **🚪 HTTP 网关** —— `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions` + `/v1/responses`（会话续接）、两者均支持 `stream: true` SSE 流式（令牌增量、工具进度/函数调用事件）、带 SSE 事件 + 审批处理的异步 `/v1/runs`、会话 API（含 `PATCH`/fork + 逐轮生效的会话级模型锁）、`/api/jobs` 定时任务管理（增删查改 + pause/resume/run，内置调度器自动执行到期任务）、`/v1/skills` + `/v1/toolsets` 发现端点、`/api/model/options`（models.dev 目录增强）、Prometheus `/metrics`、令牌核算 `/api/usage`、后台委派登记 `/v1/delegations`、浏览器 CDP 实时控制 `/v1/browser/status|connect|disconnect`、Bearer 鉴权
- **🖥️ 终端环境** —— `terminal` 可在本地（默认）、docker（自动创建容器）或 ssh 上执行（`[terminal] backend`）
- **🩺 终端失败提示** —— 失败命令自带可执行提示：良性退出码会被解释（`grep=1` → "无匹配（非错误）"，`exit_code_meaning`），常见失败形态附加一条恢复建议（`hint`）：命令/模块未找到、git 冲突、gh 字段漂移与限流、权限错误、退出码 124/126/137
- **🔬 环境探针** —— 终端后端为本地时，向系统提示注入一行确定性的 Python 工具链说明（pip/python3 版本错配、缺 pip 模块、PEP 668、缺少裸 `python`）；健康环境保持静默，后台探测 + 超时即放行（`[agent] environment_probe`）
- **🖥️ 桌面桥接工具** —— 面向 GUI 宿主的 `close_terminal` / `read_terminal` / `focus_pane` / `open_preview` / `react_to_message`（表情回应，`[display] message_reactions`）：`ULNCLAW_DESKTOP=1` 门控，经 `desktop` 桥接层路由（宿主安装的发射器按 UI 会话收到 `terminal.close` / `pane.reveal` / `preview.open` 事件）；从不杀进程，未接入宿主时返回 "desktop only"
- **🧹 ANSI 剥离** —— terminal/execute_code 输出在送达模型前清除 ECMA-48 转义序列（颜色、光标移动、OSC 标题、8-bit C1），转义序列不会泄漏进上下文或文件写入
- **🔒 沙箱凭证清洗** —— terminal/execute_code 子进程的环境中剥离 provider 与工具凭证（hermes GHSA-rhgp-j443-p4rf 语义）；技能 `required_environment_variables` 与 `[terminal] env_passthrough` 允许其余变量通过——provider 凭证永不可被放行
- **🚫 二进制守卫** —— `read_file` 拒绝约 80 种二进制扩展（图像、压缩包、可执行文件、字体、字节码、数据库），并提示改用 vision_analyze/terminal；`.pdf` 保持可读
- **📏 可配置输出上限** —— `[tool_output] max_bytes/max_lines/max_line_length` 无需改源码即可调整 terminal 截断、read_file 分页与每行截断上限
- **🕵️ 密钥脱敏** —— 约 55 种厂商密钥前缀、JWT、私钥、数据库连接串、认证头与 env 转储的 `KEY=value` 在输出送达模型前脱敏；文件内容使用不可复用哨兵，截断密钥永不会被写回
- **📸 检查点** —— 文件编辑前的透明 git 快照（共享 shadow 存储、按项目快照链），`ulnclaw checkpoints list/restore/diff/prune`
- **📝 工作区 diff** —— `ulnclaw diff [--staged|--all]` 显示 git 工作区变更（含未跟踪文件），REPL `/gitdiff`
- **🌐 Provider** —— OpenAI 兼容端点（OpenAI、OpenRouter、DashScope、Ollama、llama.cpp）+ 原生 Anthropic Messages API provider（tool_use/tool_result 块、SSE 流式、OAuth bearer），本地 provider 免密钥；按任务辅助路由（`[auxiliary.compression]`、`[auxiliary.vision]`、`[auxiliary.title_generation]`）可将二次调用发往不同 provider/模型；`[model] fallbacks` 回退链（按轮恢复主 provider）
- **📡 消息平台** —— Telegram/Discord/Slack/Signal 适配器运行于 `ulnclaw gateway` 内（`[messaging.*]`，Signal 经 signal-cli HTTP 守护进程），WhatsApp Cloud 与 Microsoft Graph 接入挂载为网关 webhook 路由（`/webhooks/whatsapp` HMAC 校验、`/webhooks/msgraph` clientState 校验），另有通用签名 webhook 平台（`[messaging.webhook]` 路由 `/webhooks/hook/<name>` —— Svix/GitHub/GitLab/HMAC-V2 签名方案、每路由限流、投递 id 幂等、`deliver_only` 零 LLM 推送）、BlueBubbles iMessage（`[messaging.bluebubbles]` → `/webhooks/bluebubbles` 密码校验 webhook、LRU 缓存的 chat-GUID 解析、REST 文本/附件发送）、微信个人号（`[messaging.weixin]` —— 腾讯 iLink Bot API 长轮询、`ulnclaw weixin login` 扫码登录、双向 AES-128-ECB 加密 CDN 媒体、context_token 回显发送）、QQ（`[messaging.qq]` —— 官方 QQ Bot API v2 WebSocket 网关 + REST、markdown 回复、分块媒体上传、`asr_refer_text` 语音转写）：白名单配对（fail closed）+ hermes 风格交互式配对码（`pairing list/approve/revoke/clear-pending`）、媒体附件缓存于 `media-cache/` 并以路径引用交付（出站 `MEDIA:` 标签在 Telegram/Discord/Slack 原生上传；WhatsApp 媒体双向经 Graph `/media` 端点）、每聊天一条持久会话、hermes 风格回复分块。`clarify` 工具在聊天中可用：WhatsApp 渲染原生按钮/列表，其余平台编号文本；点按与后续文本应答待决提问
- **🎙️ 语音转写（STT）** —— 入站语音/音频消息在 agent 回合前转写（`[stt]` 配置）：内置 `local_command` / `groq` / `openai` / `mistral` / `xai` / `elevenlabs` / `deepinfra` provider，另支持自定义 `[stt.providers.<name>]` 命令 provider，转写文本以 🎙️ 消息回显并注入回合（复刻 hermes 回退/哨兵语义）；`transcribe_audio` 工具（可选 `stt` 工具集）覆盖任意音频文件。Python 专属的 faster-whisper `local` provider 以 `stt.local.command` / 云后端替代
- **📋 Kanban 引擎** —— `ulnclaw kanban`：`kanban.db` 中的多看板任务引擎（hermes 状态 todo/ready/running/scheduled/blocked/done/archived，带图标），带 TTL 的认领锁 + 过期接管与心跳、评论与事件轨迹、看板增删改查；生命周期流转触发 `kanban_task_*` 插件钩子
- **🔌 插件与钩子** —— 目录插件（`~/.ulnclaw/plugins/<name>/plugin.toml`：hooks + 子进程工具）与 `[hooks]` 配置式 shell 钩子，复刻 hermes 首次使用同意机制（`plugins list/enable/disable/accept-hooks`、`hooks list/test/revoke/doctor`）；核心触发 hermes 运行期实际发出的全部 13 个钩子事件（工具/LLM 前后、API 请求生命周期、会话边界、网关分发门控）
- **🔑 Secrets 保险库** —— 外部秘密源在启动时、provider 读取 env 之前应用（`secrets status/sync`）：command 助手、Bitwarden Secrets Manager（`bws` —— 固定版本自动安装、AES-GCM 加密 TTL 缓存、`secrets bitwarden setup` 向导）、1Password（`op://` 引用、`secrets onepassword setup/set`），完整复刻 hermes 优先级语义
- **🖱️ Computer use** —— `computer_use` 工具经 cua-driver 守护进程（MCP over stdio，完整 hermes schema），与 hermes 相同的审批门控；`computer-use status/doctor/install`
- **🔄 OAuth + 技能同步** —— `auth login` 对任意 `[oauth]` provider 执行 RFC 8628 设备流；`sync status/pull/push/now` 经 HTTP(S) 或共享目录同步技能
- **🖥️ 桌面 GUI** —— `desktop/`：Tauri 2 外壳（取代 hermes 的 Electron 应用），承载聊天界面（会话重命名/删除悬停操作、`hermes.tool.progress` SSE 事件驱动的实时工具进度条、网关命令 + 已装技能的 `/` 斜杠补全弹层、`hermes.tool.started/completed` SSE 事件驱动的可展开工具调用卡片、剪贴板图片粘贴经 `POST /api/uploads` 上传并以媒体路径引用附加）并管理 gateway 子进程；网关内置的本地应用 CORS 同样服务任意浏览器仪表盘

### CLI 快速开始

```bash
cargo build --release --target x86_64-unknown-linux-musl

# 生成默认配置 ~/.ulnclaw/config.toml
./ulnclaw init

# 一次性运行
./ulnclaw run "总结一下 README.md"

# 交互式聊天（斜杠命令：/new /search /memory /skills /sessions /rollback /diff /recap /goal /subgoal /focus /verbose /stash /paste ……）
./ulnclaw chat
./ulnclaw chat --resume <session-id>   # 按 id 或唯一前缀恢复会话（-r）
./ulnclaw chat --continue              # 继续最近一次会话（-c）

# 管理子命令
./ulnclaw tools            # 列出工具集与已启用工具
./ulnclaw sessions list    # state.db 中的最近会话
./ulnclaw sessions search "认证重构"
./ulnclaw sessions export <session-id> --out ./exports --format md|html
./ulnclaw sessions recover ./damaged-state.db   # 离线数据库恢复
./ulnclaw sessions repair          # 修复受损 state.db 库结构（--check-only）
./ulnclaw sessions browse          # 交互式会话挑选：过滤并恢复会话
./ulnclaw sessions retitle-skills  # 修复泄漏 /skill 脚手架的会话标题（--apply）
./ulnclaw sessions delete|rename|optimize # 单会话删除/重命名；FTS 合并 + VACUUM 回收空间
./ulnclaw skills list
./ulnclaw skills blueprints    # 可排程技能（skills schedule <name>）
./ulnclaw skills scan <name>   # 信任技能前的安全扫描（--json、--source、--force）
./ulnclaw cron list          # cron jobs (cron run <id> executes one immediately)
./ulnclaw suggestions        # suggested automations (accept/dismiss/catalog/clear)
./ulnclaw moa list           # MoA 预设（运行：./ulnclaw moa run "<prompt>"）
./ulnclaw models providers   # models.dev 目录（list/info/refresh）
./ulnclaw checkpoints list   # 文件系统快照（[checkpoints] enabled = true）
./ulnclaw diff               # git 工作区 diff（--staged / --all）
./ulnclaw doctor             # 诊断配置与依赖（--fix、--online、--json）
./ulnclaw insights           # 会话用量分析（--days、--source、--json）
./ulnclaw status             # 全组件状态总览（--deep）
./ulnclaw logs               # 查看/过滤日志（-f、-n、--level、--since、--component）
./ulnclaw update --check   # 检查更新（ulnclaw update 应用：stash -> ff 拉取 -> 重建）
./ulnclaw config           # 配置 show/get/set/unset（env 风格键写入 .env）
./ulnclaw secrets status   # 外部秘密源（secrets sync [--apply] 立即拉取）
./ulnclaw secrets bitwarden setup   # 向导：安装 bws、存令牌、选项目（另有 install/status/token/disable；onepassword setup/status/set/remove/disable）
./ulnclaw computer-use status # cua-driver 后台桌面控制（doctor/install）
./ulnclaw plugins list      # 插件与 shell 钩子（enable/disable/accept-hooks）
./ulnclaw kanban list       # kanban 任务引擎（init/boards/create/claim/done/block/comment/...）
./ulnclaw hooks doctor      # 逐个探测已同意的钩子（list/test/revoke）
./ulnclaw pairing list      # 陌生发送者的 DM 配对码（approve/revoke/clear-pending）
./ulnclaw weixin login      # 微信 iLink 扫码登录（[messaging.weixin]）
./ulnclaw auth login        # OAuth 设备流登录（status/refresh/logout/open）
./ulnclaw sync status       # 技能同步（pull/push/now/enable/disable/device）
./ulnclaw completion bash  # shell 补全脚本（bash/zsh/fish/elvish/powershell）
./ulnclaw dump             # 可粘贴的装机摘要，用于求助排查（--show-keys）
./ulnclaw version          # 版本 + 安装信息 + 升级状态
./ulnclaw uninstall        # 移除代码/PATH 条目/包装脚本（--full 连数据清除、--dry-run、--yes）
./ulnclaw memory           # 持久记忆状态；`memory reset [all|memory|user]` 清除
./ulnclaw approvals        # 终端审批模式；`approvals manual|smart|off` 设置
./ulnclaw prompt-size      # 系统提示词 + 工具 schema 体积（--json）
./ulnclaw debug report     # 脱敏诊断包，用于求助分享（--no-redact）
./ulnclaw bundles          # 技能束：一个 /命令 加载一组技能
./ulnclaw import-agent     # 导入 Claude Code / Codex 配置（--dry-run）
./ulnclaw security audit   # 固定版本 MCP 包的 OSV.dev 审计（--json）
./ulnclaw fallback         # 回退链管理（add/remove/clear provider:model 条目）
./ulnclaw backup           # home 目录 zip 备份（-q 快速快照，backup list/restore/prune）
./ulnclaw import b.zip     # 恢复备份 zip（跳过运行时状态文件，机密文件 0600）

# 浏览器自动化：auto 模式自动启动托管的无头 Chrome/Chromium；
# 也可将 browser_* 工具指向已开启远程调试的浏览器
export ULNCLAW_BROWSER_CDP=http://127.0.0.1:9222     # 或 ws://.../devtools/browser/... 或 "auto"

# HTTP 网关（OpenAI 兼容 API 服务器，默认 127.0.0.1:8642）
./ulnclaw gateway --host 127.0.0.1 --port 8642
# 消息平台随网关运行（[messaging.telegram|discord|slack]）
# Tauri 桌面应用（desktop/）与任意浏览器仪表盘经 HTTP/SSE 连接本网关
# （内置本地应用 CORS，见 desktop/README.md）
curl -H "Authorization: Bearer $ULNCLAW_GATEWAY_KEY" \\
     -H "Content-Type: application/json" \\
     -d '{"messages":[{"role":"user","content":"你好！"}]}' \\
     http://127.0.0.1:8642/v1/chat/completions
```

### 库用法快速开始

```rust
use ulnclaw::prelude::*;
use ulnclaw::{register_builtin_tools, ToolRegistry, SqliteSessionStore};

#[tokio::main]
async fn main() -> Result<()> {
    let provider = OpenAiProvider::builder()
        .endpoint("http://localhost:11434/v1")
        .model("qwen3:32b")
        .build()?;

    let mut tools = ToolRegistry::new();
    register_builtin_tools(&mut tools);          // 全部 50+ hermes 风格工具

    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig { approval: false, ..Default::default() })
        .with_store(Arc::new(SqliteSessionStore::open_default()?));

    println!("{}", agent.chat("列出当前目录的文件").await?);
    Ok(())
}
```

### 文档

- [Hermes 对标矩阵](docs/zh/hermes-parity.md) —— 与 hermes-agent v2026.8.3 的工具/功能逐项对应
- [架构指南](docs/zh/architecture.md) · [API 参考](docs/zh/api-reference.md)
- [集成指南](docs/zh/integration.md) · [开发指南](docs/zh/development.md)
- [工具系统](docs/zh/tools.md) · [Provider 系统](docs/zh/providers.md)

### 构建与测试

```bash
cargo test                     # 893 个测试
cargo build --release --target x86_64-unknown-linux-musl   # 静态二进制
```

### 许可证

MIT OR Apache-2.0
