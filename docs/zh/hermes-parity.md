# Hermes Agent 对标矩阵 (v2026.8.3)

本文档跟踪 ulnclaw 与
[hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3)
的对应关系。ulnclaw 是 hermes agent 引擎的 Rust 重实现：相同的工具面、
相同的存储布局、相同的配置语义 —— 原生性能，单一静态二进制。

## 工具对标

| hermes 工具 | ulnclaw | 说明 |
|---|---|---|
| `terminal`, `process` | ✅ 完整 | 前台/后台执行、超时、工作目录跟踪、后台会话管理（list/log/wait/kill） |
| `read_file`, `write_file`, `patch`, `search_files` | ✅ 完整 | 带行号读取与 `next_offset` 分页、模糊替换（容忍空白/缩进差异）、V4A 多文件补丁、unified diff、ripgrep 风格搜索 |
| `web_search`, `web_extract` | ✅ 完整 | 可插拔后端：Tavily / Brave / SearXNG / 内置 DuckDuckGo；HTML→文本抽取 |
| `memory` | ✅ 完整 | `MEMORY.md` + `USER.md`，原子批量 `operations`，字符上限（2200/1375），每轮注入系统提示词 |
| `todo` | ✅ 完整 | 会话任务列表、merge 模式、强制单一 `in_progress` |
| `session_search` | ✅ 完整 | SQLite FTS5 检索 + 会话内滚动，会话血缘 |
| `clarify` | ✅ 完整 | 单选/多选/开放式提问（经前端回调） |
| `skills_list`, `skill_view`, `skill_manage` | ✅ 完整 | SKILL.md frontmatter、关联文件（references/templates/scripts）、路径穿越防护 |
| `delegate_task` | ✅ 完整 | 并行子代理、深度限制、隔离上下文、子会话 |
| `execute_code` | ✅ 完整 | python3 子进程沙箱，120 秒上限 |
| `cronjob` | ✅ 完整 | create/list/update/pause/resume/remove/run；`30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划；SQLite 任务存储；调度循环 |
| `tool_search` | ✅ 完整 | 按关键词搜索已注册工具目录 |
| `vision_analyze` | ✅ 完整 | 经聊天 provider 的 `analyze_image` 路由 |
| `image_generate` | ✅ 完整 | OpenAI images API，PNG 存于 `<home>/images` |
| `text_to_speech` | ✅ 完整 | OpenAI TTS 或自定义 `ULNCLAW_TTS_ENDPOINT` |
| `ha_*`（4 个 Home Assistant 工具） | ✅ 完整 | Home Assistant REST API，依赖 `HASS_URL` + `HASS_TOKEN` |
| `kanban_*`（12 个工具） | ✅ 完整 | 本地 SQLite 协作看板：create/list/show/complete/block/unblock/comment/heartbeat/link/attach/attach_url/attachments |
| `browser_*`（12 个工具） | ✅ 完整 | CDP WebSocket 客户端（`browser` 模块）：端点发现、页面会话、带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框；门控于 `ULNCLAW_BROWSER_CDP`（ws:// 或 http://host:port） |
| `computer_use` | 🟡 门控 | 需要 computer-use 驱动（hermes：cua-driver） |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 门控 | 已注册，依赖平台凭据门控；后端待实现 |
| `x_search`, `video_analyze`, `video_generate`, `bfl_flux3_*` | ⬜ 暂缓 | 依赖特定供应商（xAI/BFL）；凭据就绪后再补 |

## 功能对标

| hermes 功能 | ulnclaw | 说明 |
|---|---|---|
| 工具调用代理循环 | ✅ | 迭代预算、用量统计、step 回调 |
| SQLite 状态库（`hermes_state.py`） | ✅ | sessions/messages/system_prompts/state_meta/async_delegations 表结构，FTS5（不可用时 LIKE 回退），会话血缘 |
| 上下文压缩（`conversation_compression.py`） | ✅ | 预算触发，中段对话经二次模型调用摘要，保留系统提示词 + 首条用户消息 + 最近尾部 |
| 审批系统（`approval.py`） | ✅ 核心 | 命令归一化（反斜杠续行、`${IFS}`、注释剥离）、硬性底线（直接阻止）、可恢复但昂贵的操作（需确认） |
| 威胁模式扫描（`threat_patterns.py`） | ✅ 核心 | 对重新进入上下文的工具结果做提示注入扫描（建议性） |
| 工具集（`toolsets.py`） | ✅ | 全部 33 个工具集定义，含组合（`includes`），默认 `coding` |
| 工具注册表（`registry.py`） | ✅ | check_fn 门控、工具集分组、结果大小截断 |
| Provider 抽象（`runtime_provider.py`） | ✅ | OpenAI 兼容（OpenAI/OpenRouter/DashScope/Ollama/llama.cpp），本地 provider 免密钥 |
| 配置（`config.yaml`） | ✅ | `config.toml` + `.env` 文件、profiles、环境变量优先级 |
| 技能系统 | ✅ | 发现、frontmatter、关联文件 |
| 记忆系统 | ✅ | MEMORY.md/USER.md，注入提示词 |
| Cron 调度器 | ✅ | 任务存储 + 计划解析 + 轮询循环（`cron::run_scheduler`） |
| MCP 客户端（`mcp_tool.py`） | ✅ 核心 | stdio JSON-RPC：initialize/tools/list/tools/call；`[[mcp.servers]]` 配置；工具注册为 `mcp__<server>__<tool>` |
| CLI（`hermes_cli/`） | ✅ 核心 | 带斜杠命令的聊天 REPL、一次性 `run`、sessions/tools/skills/cron 子命令、`init` |
| 委派（delegation） | ✅ | SubAgentRunner trait、深度限制、子会话 |
| HTTP 网关（`gateway/platforms/api_server.py`） | ✅ 核心 | `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions`（`X-Ulnclaw-Session-Id` 会话续接）、`/v1/models`、`/v1/capabilities`、`/v1/runs`（异步运行 + 停止）、`/api/sessions` 增删查改 + 会话聊天、Bearer 令牌鉴权 |
| 消息平台（Telegram/WhatsApp/QQ 等） | ⬜ 暂缓 | hermes 的平台适配器未移植；HTTP 网关已覆盖 OpenAI 兼容前端 |
| TUI/web/app | ⬜ 暂缓 | hermes 提供 TUI 与 web 应用；ulnclaw 目前是库 + CLI + HTTP 网关 |
| 环境（docker/ssh/modal/daytona/vercel） | ⬜ 暂缓 | terminal 在本地执行；远程后端待做 |
| 检查点管理器、浏览器监督器、CUA | ⬜ 暂缓 | 重量级子系统 |

## 存储布局

```
~/.ulnclaw/                 （支持 ULNCLAW_HOME 覆盖；兼容 HERMES_HOME 以便迁移）
├── config.toml             主配置
├── .env                    KEY=VALUE 密钥（进程环境变量优先）
├── state.db                SQLite：sessions、messages、cron_jobs、meta（+FTS5）
├── kanban.db               kanban 看板
├── memory/MEMORY.md        代理记忆
├── memory/USER.md          用户画像
├── skills/<name>/SKILL.md  技能
├── sessions/*.todos.json   每会话 todo 列表
├── images/  audio/         生成的产物
└── sandboxes/              execute_code 脚本
```

## 已知差异

- 审批交互为终端 y/N 提示（hermes 有更丰富的平台化流程）。
- 浏览器工具需要可达的、开启远程调试的 Chrome/Chromium（`ULNCLAW_BROWSER_CDP`）；尚无托管浏览器启动（hermes 的 browser supervisor）。
- 网关实现了 api_server 平台的子集；SSE 运行事件、`/v1/responses`、多 profile 复用未移植。
- 压缩使用 字符数/4 的 token 估算而非分词器。
- `patch` 模糊链实现了 hermes 的 4 个确定性策略
  （精确 → 行尾修剪 → 空白归一 → 缩进自适应）；两个基于相似度的策略
  （block_anchor、context_aware）未移植。
