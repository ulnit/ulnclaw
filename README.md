# ulnclaw 🦞

[English](#english) | [中文](#中文)

---

## English

**A high-performance AI agent engine written in Rust — a port of [hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3)**

ulnclaw re-implements the Hermes Agent engine in Rust: the same tool surface
(50+ built-in tools), the same SQLite session/memory/skills/cron storage
layout, the same toolset composition — with native performance and a single
static musl binary. See the [parity matrix](docs/en/hermes-parity.md) for the
full feature-by-feature mapping.

### Key Features

- **🤖 Agent loop** — tool calling with iteration budgets, usage accounting, memory injection, step/tool callbacks
- **🔧 50+ built-in tools** — terminal/process, file read/write/patch/search, web search/extract, memory, todo, session search, clarify, skills, delegation, execute_code, cronjob, vision, image generation, TTS, Home Assistant, kanban, tool search
- **🧰 Toolsets** — hermes-compatible grouping (`coding`, `web`, `file`, `safe`, `debugging`, ...) with composition and enable/disable policy
- **🛡️ Approval system** — command normalization, hardline floor (auto-block), confirm-before-run for costly operations; REPL prompts plus gateway run approvals over HTTP with fail-closed timeout and persisted `always` grants
- **💾 SQLite state** — sessions/messages with FTS5 full-text search, lineage (parent/child sessions), cron jobs, kanban board; offline non-destructive `sessions recover` for damaged databases (rowid salvage, orphan-session reconstruction, FTS rebuild)
- **🗜️ Context compression** — budget-triggered middle-turn summarization via a secondary model call
- **🤝 Delegation** — parallel sub-agents with isolated contexts and depth limits
- **🧬 Mixture of Agents** — `[moa]` presets fan a prompt out to reference models in parallel and synthesize their answers via an aggregator (`ulnclaw moa run/list/delete`, REPL `/moa`)
- **⏰ Cron** — `30m` / `every 2h` / `0 9 * * *` / ISO one-shot schedules with a poll scheduler
- **📐 Blueprints** — skills with a `metadata.hermes.blueprint.schedule` frontmatter become cron jobs (`skills blueprints`, `skills schedule/unschedule`)
- **🔌 MCP client** — stdio JSON-RPC: any MCP server's tools appear as `mcp__<server>__<tool>`; npx/uvx launches get an OSV malware preflight (MAL-* advisories block, fail-open)
- **🌍 Browser automation** — CDP WebSocket client with a built-in supervisor (auto-launches headless Chrome/Chromium, or point `ULNCLAW_BROWSER_CDP` at your own): accessibility snapshots with element refs, click/type/scroll/press/screenshot/JS evaluate/dialogs
- **🚪 HTTP gateway** — `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` + `/v1/responses` (with session continuity), `stream: true` SSE streaming on both (token deltas, tool-progress/function-call events), async `/v1/runs` with SSE events + approval resolution, sessions API (incl. `PATCH`/fork + enforced per-session model lock), `/api/jobs` cron management (CRUD + pause/resume/run), `/v1/skills` + `/v1/toolsets` discovery, `/api/model/options`, Prometheus `/metrics`, token accounting `/api/usage`, bearer auth
- **🖥️ Terminal environments** — run `terminal` locally (default), in docker (auto container creation), or over ssh (`[terminal] backend`)
- **🩺 Terminal failure intelligence** — failed commands carry actionable guidance: benign exit codes are explained (`grep=1` → "no matches, not an error" as `exit_code_meaning`) and well-known failure shapes get one recovery `hint` (command/module not found, git conflicts, gh field drift & rate limits, permission errors, exits 124/126/137)
- **🔬 Environment probe** — when the terminal backend is local, one deterministic Python-toolchain line (pip/python3 mismatch, missing pip module, PEP 668, missing bare `python`) is injected into the system prompt — silent on healthy machines, background-probed with fail-open timeout (`[agent] environment_probe`)
- **🖥️ Desktop bridge tools** — `close_terminal` / `read_terminal` / `focus_pane` / `open_preview` for GUI hosts: gated on `ULNCLAW_DESKTOP=1`, routed through the `desktop` bridge (host-installed emitter receives `terminal.close` / `pane.reveal` / `preview.open` events per UI session); never kill processes, report "desktop only" without a wired host
- **🧹 ANSI stripping** — terminal/execute_code output is cleaned of ECMA-48 escape sequences (colors, cursor moves, OSC titles, 8-bit C1) before it reaches the model, so escapes never leak into context or file writes
- **🔒 Sandbox credential scrub** — terminal/execute_code child processes run with provider & tool credentials stripped from their environment (hermes GHSA-rhgp-j443-p4rf semantics); skills' `required_environment_variables` and `[terminal] env_passthrough` allowlist the rest — provider credentials can never be allowlisted
- **📸 Checkpoints** — transparent git-backed snapshots before file edits (shared shadow store, per-project chains), `ulnclaw checkpoints list/restore/diff/prune`
- **📝 Working diff** — `ulnclaw diff [--staged|--all]` shows what changed in a git worktree (untracked files included), REPL `/gitdiff`
- **🌐 Providers** — OpenAI-compatible endpoints (OpenAI, OpenRouter, DashScope, Ollama, llama.cpp) plus a native Anthropic Messages API provider (tool_use/tool_result blocks, SSE streaming, OAuth bearer), keyless local providers; per-task auxiliary routing (`[auxiliary.compression]`, `[auxiliary.vision]`) sends secondary calls to a different provider/model; `[model] fallbacks` failover chain with per-turn primary restore

### CLI Quick Start

```bash
cargo build --release --target x86_64-unknown-linux-musl

# Write a default config to ~/.ulnclaw/config.toml
./ulnclaw init

# One-shot run
./ulnclaw run "Summarize the README.md file"

# Interactive chat (slash commands: /new /search /memory /skills /sessions /rollback /diff /recap ...)
./ulnclaw chat

# Management subcommands
./ulnclaw tools            # list toolsets and enabled tools
./ulnclaw sessions list    # recent sessions from state.db
./ulnclaw sessions search "auth refactor"
./ulnclaw sessions export <session-id> --out ./exports --format md|html
./ulnclaw sessions recover ./damaged-state.db   # offline db recovery
./ulnclaw skills list
./ulnclaw skills blueprints    # schedulable skills (skills schedule <name>)
./ulnclaw cron list
./ulnclaw moa list           # MoA presets (run: ./ulnclaw moa run "<prompt>")
./ulnclaw checkpoints list   # filesystem snapshots ([checkpoints] enabled = true)
./ulnclaw diff               # git working-tree diff (--staged / --all)

# Browser automation: auto mode launches a managed headless Chrome/Chromium;
# or point browser_* tools at an existing browser with remote debugging
export ULNCLAW_BROWSER_CDP=http://127.0.0.1:9222     # or ws://.../devtools/browser/... or "auto"

# HTTP gateway (OpenAI-compatible API server, default 127.0.0.1:8642)
./ulnclaw gateway --host 127.0.0.1 --port 8642
curl -H "Authorization: Bearer $ULNCLAW_GATEWAY_KEY" \\
     -H "Content-Type: application/json" \\
     -d '{"messages":[{"role":"user","content":"Hello!"}]}' \\
     http://127.0.0.1:8642/v1/chat/completions
```

Example `~/.ulnclaw/config.toml`:

```toml
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

# [terminal]
# backend = "docker"    # "local" (default) | "docker" | "ssh"
# container = "ulnclaw-dev"
# image = "ubuntu:24.04"
# env_passthrough = ["TENOR_API_KEY"]   # vars allowed past the sandbox credential scrub

# [checkpoints]
# enabled = true        # transparent snapshots before write_file/patch

# Auxiliary model routing: run secondary calls on a different model.
# [auxiliary.compression]   # context-compression summaries
# provider = "openai"
# model = "gpt-5.2-mini"
# [auxiliary.vision]        # image analysis (vision_analyze / browser_vision)
# model = "gpt-5.2"

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
cargo test                     # 232 tests
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
[对标矩阵](docs/zh/hermes-parity.md)。

### 核心特性

- **🤖 Agent 循环** —— 工具调用、迭代预算、用量统计、记忆注入、步骤/工具回调
- **🔧 50+ 内置工具** —— terminal/process、文件读/写/补丁/搜索、web 搜索/抽取、记忆、todo、会话搜索、clarify、技能、委派、execute_code、cronjob、视觉、图像生成、TTS、Home Assistant、kanban、工具搜索
- **🧰 工具集** —— hermes 兼容分组（`coding`、`web`、`file`、`safe`、`debugging`……），支持组合与启用/禁用策略
- **🛡️ 审批系统** —— 命令归一化、硬性底线（自动阻止）、高成本操作先确认再执行；REPL 提示 + 网关 HTTP 运行审批（fail-closed 超时、`always` 授权持久化）
- **💾 SQLite 状态库** —— 会话/消息 FTS5 全文检索、会话血缘（父子会话）、定时任务、kanban 看板；受损数据库的离线非破坏性 `sessions recover`（rowid 抢救、孤儿会话重建、FTS 重建）
- **🗜️ 上下文压缩** —— 预算触发，中段对话经二次模型调用摘要
- **🤝 委派** —— 并行子代理，隔离上下文，深度限制
- **🧬 混合智能体（MoA）** —— `[moa]` 预设将提示词并行扇出给参考模型，经聚合器综合（`ulnclaw moa run/list/delete`、REPL `/moa`）
- **⏰ 定时任务** —— `30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划 + 轮询调度器
- **📐 蓝图（Blueprints）** —— frontmatter 声明 `metadata.hermes.blueprint.schedule` 的技能可排程（`skills blueprints`、`skills schedule/unschedule`）
- **🔌 MCP 客户端** —— stdio JSON-RPC：任意 MCP 服务器的工具以 `mcp__<server>__<tool>` 注册；npx/uvx 启动前经 OSV 恶意软件检查（MAL-* 通告阻止、fail-open）
- **🌍 浏览器自动化** —— CDP WebSocket 客户端 + 内置监督器（自动启动无头 Chrome/Chromium，或用 `ULNCLAW_BROWSER_CDP` 指向自有浏览器）：带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框
- **🚪 HTTP 网关** —— `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions` + `/v1/responses`（会话续接）、两者均支持 `stream: true` SSE 流式（令牌增量、工具进度/函数调用事件）、带 SSE 事件 + 审批处理的异步 `/v1/runs`、会话 API（含 `PATCH`/fork + 逐轮生效的会话级模型锁）、`/api/jobs` 定时任务管理（增删查改 + pause/resume/run）、`/v1/skills` + `/v1/toolsets` 发现端点、`/api/model/options`、Prometheus `/metrics`、令牌核算 `/api/usage`、Bearer 鉴权
- **🖥️ 终端环境** —— `terminal` 可在本地（默认）、docker（自动创建容器）或 ssh 上执行（`[terminal] backend`）
- **🩺 终端失败提示** —— 失败命令自带可执行提示：良性退出码会被解释（`grep=1` → "无匹配（非错误）"，`exit_code_meaning`），常见失败形态附加一条恢复建议（`hint`）：命令/模块未找到、git 冲突、gh 字段漂移与限流、权限错误、退出码 124/126/137
- **🔬 环境探针** —— 终端后端为本地时，向系统提示注入一行确定性的 Python 工具链说明（pip/python3 版本错配、缺 pip 模块、PEP 668、缺少裸 `python`）；健康环境保持静默，后台探测 + 超时即放行（`[agent] environment_probe`）
- **🖥️ 桌面桥接工具** —— 面向 GUI 宿主的 `close_terminal` / `read_terminal` / `focus_pane` / `open_preview`：`ULNCLAW_DESKTOP=1` 门控，经 `desktop` 桥接层路由（宿主安装的发射器按 UI 会话收到 `terminal.close` / `pane.reveal` / `preview.open` 事件）；从不杀进程，未接入宿主时返回 "desktop only"
- **🧹 ANSI 剥离** —— terminal/execute_code 输出在送达模型前清除 ECMA-48 转义序列（颜色、光标移动、OSC 标题、8-bit C1），转义序列不会泄漏进上下文或文件写入
- **🔒 沙箱凭证清洗** —— terminal/execute_code 子进程的环境中剥离 provider 与工具凭证（hermes GHSA-rhgp-j443-p4rf 语义）；技能 `required_environment_variables` 与 `[terminal] env_passthrough` 允许其余变量通过——provider 凭证永不可被放行
- **📸 检查点** —— 文件编辑前的透明 git 快照（共享 shadow 存储、按项目快照链），`ulnclaw checkpoints list/restore/diff/prune`
- **📝 工作区 diff** —— `ulnclaw diff [--staged|--all]` 显示 git 工作区变更（含未跟踪文件），REPL `/gitdiff`
- **🌐 Provider** —— OpenAI 兼容端点（OpenAI、OpenRouter、DashScope、Ollama、llama.cpp）+ 原生 Anthropic Messages API provider（tool_use/tool_result 块、SSE 流式、OAuth bearer），本地 provider 免密钥；按任务辅助路由（`[auxiliary.compression]`、`[auxiliary.vision]`）可将二次调用发往不同 provider/模型；`[model] fallbacks` 回退链（按轮恢复主 provider）

### CLI 快速开始

```bash
cargo build --release --target x86_64-unknown-linux-musl

# 生成默认配置 ~/.ulnclaw/config.toml
./ulnclaw init

# 一次性运行
./ulnclaw run "总结一下 README.md"

# 交互式聊天（斜杠命令：/new /search /memory /skills /sessions /rollback /diff /recap ……）
./ulnclaw chat

# 管理子命令
./ulnclaw tools            # 列出工具集与已启用工具
./ulnclaw sessions list    # state.db 中的最近会话
./ulnclaw sessions search "认证重构"
./ulnclaw sessions export <session-id> --out ./exports --format md|html
./ulnclaw sessions recover ./damaged-state.db   # 离线数据库恢复
./ulnclaw skills list
./ulnclaw skills blueprints    # 可排程技能（skills schedule <name>）
./ulnclaw cron list
./ulnclaw moa list           # MoA 预设（运行：./ulnclaw moa run "<prompt>"）
./ulnclaw checkpoints list   # 文件系统快照（[checkpoints] enabled = true）
./ulnclaw diff               # git 工作区 diff（--staged / --all）

# 浏览器自动化：auto 模式自动启动托管的无头 Chrome/Chromium；
# 也可将 browser_* 工具指向已开启远程调试的浏览器
export ULNCLAW_BROWSER_CDP=http://127.0.0.1:9222     # 或 ws://.../devtools/browser/... 或 "auto"

# HTTP 网关（OpenAI 兼容 API 服务器，默认 127.0.0.1:8642）
./ulnclaw gateway --host 127.0.0.1 --port 8642
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
cargo test                     # 232 个测试
cargo build --release --target x86_64-unknown-linux-musl   # 静态二进制
```

### 许可证

MIT OR Apache-2.0
