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
- **💾 SQLite state** — sessions/messages with FTS5 full-text search, lineage (parent/child sessions), cron jobs, kanban board
- **🗜️ Context compression** — budget-triggered middle-turn summarization via a secondary model call
- **🤝 Delegation** — parallel sub-agents with isolated contexts and depth limits
- **⏰ Cron** — `30m` / `every 2h` / `0 9 * * *` / ISO one-shot schedules with a poll scheduler
- **🔌 MCP client** — stdio JSON-RPC: any MCP server's tools appear as `mcp__<server>__<tool>`
- **🌍 Browser automation** — CDP WebSocket client with a built-in supervisor (auto-launches headless Chrome/Chromium, or point `ULNCLAW_BROWSER_CDP` at your own): accessibility snapshots with element refs, click/type/scroll/press/screenshot/JS evaluate/dialogs
- **🚪 HTTP gateway** — `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` + `/v1/responses` (with session continuity), `stream: true` SSE streaming on both (token deltas, tool-progress/function-call events), async `/v1/runs` with SSE events + approval resolution, sessions API (incl. `PATCH`/fork + enforced per-session model lock), `/api/jobs` cron management (CRUD + pause/resume/run), `/v1/skills` + `/v1/toolsets` discovery, `/api/model/options`, bearer auth
- **🖥️ Terminal environments** — run `terminal` locally (default), in docker (auto container creation), or over ssh (`[terminal] backend`)
- **📸 Checkpoints** — transparent git-backed snapshots before file edits (shared shadow store, per-project chains), `ulnclaw checkpoints list/restore/diff/prune`
- **🌐 Providers** — OpenAI-compatible endpoints (OpenAI, OpenRouter, DashScope, Ollama, llama.cpp) plus a native Anthropic Messages API provider (tool_use/tool_result blocks, SSE streaming, OAuth bearer), keyless local providers; per-task auxiliary routing (`[auxiliary.compression]`, `[auxiliary.vision]`) sends secondary calls to a different provider/model

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
./ulnclaw skills list
./ulnclaw cron list
./ulnclaw checkpoints list   # filesystem snapshots ([checkpoints] enabled = true)

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

[agent]
max_iterations = 90
approval = true                # y/N prompt before dangerous commands

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

# [checkpoints]
# enabled = true        # transparent snapshots before write_file/patch

# Auxiliary model routing: run secondary calls on a different model.
# [auxiliary.compression]   # context-compression summaries
# provider = "openai"
# model = "gpt-5.2-mini"
# [auxiliary.vision]        # image analysis (vision_analyze / browser_vision)
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
cargo test                     # 139 tests
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
- **💾 SQLite 状态库** —— 会话/消息 FTS5 全文检索、会话血缘（父子会话）、定时任务、kanban 看板
- **🗜️ 上下文压缩** —— 预算触发，中段对话经二次模型调用摘要
- **🤝 委派** —— 并行子代理，隔离上下文，深度限制
- **⏰ 定时任务** —— `30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划 + 轮询调度器
- **🔌 MCP 客户端** —— stdio JSON-RPC：任意 MCP 服务器的工具以 `mcp__<server>__<tool>` 注册
- **🌍 浏览器自动化** —— CDP WebSocket 客户端 + 内置监督器（自动启动无头 Chrome/Chromium，或用 `ULNCLAW_BROWSER_CDP` 指向自有浏览器）：带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框
- **🚪 HTTP 网关** —— `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions` + `/v1/responses`（会话续接）、两者均支持 `stream: true` SSE 流式（令牌增量、工具进度/函数调用事件）、带 SSE 事件 + 审批处理的异步 `/v1/runs`、会话 API（含 `PATCH`/fork + 逐轮生效的会话级模型锁）、`/api/jobs` 定时任务管理（增删查改 + pause/resume/run）、`/v1/skills` + `/v1/toolsets` 发现端点、`/api/model/options`、Bearer 鉴权
- **🖥️ 终端环境** —— `terminal` 可在本地（默认）、docker（自动创建容器）或 ssh 上执行（`[terminal] backend`）
- **📸 检查点** —— 文件编辑前的透明 git 快照（共享 shadow 存储、按项目快照链），`ulnclaw checkpoints list/restore/diff/prune`
- **🌐 Provider** —— OpenAI 兼容端点（OpenAI、OpenRouter、DashScope、Ollama、llama.cpp）+ 原生 Anthropic Messages API provider（tool_use/tool_result 块、SSE 流式、OAuth bearer），本地 provider 免密钥；按任务辅助路由（`[auxiliary.compression]`、`[auxiliary.vision]`）可将二次调用发往不同 provider/模型

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
./ulnclaw skills list
./ulnclaw cron list
./ulnclaw checkpoints list   # 文件系统快照（[checkpoints] enabled = true）

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
cargo test                     # 139 个测试
cargo build --release --target x86_64-unknown-linux-musl   # 静态二进制
```

### 许可证

MIT OR Apache-2.0
