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
| 蓝图 Blueprints（`tools/blueprints.py`） | ✅ 核心 | 在 frontmatter 声明 `metadata.hermes.blueprint.schedule` 的技能可排程：`skills blueprints`（列表）、`skills schedule <name>`（创建 `blueprint:<skill>` 定时任务并挂载技能）、`skills unschedule <name>`；畸形 blueprint 块显式报错；`skills list` 以日程标注蓝图。hermes 的建议队列与 `export_blueprint` 发布路径未移植（以显式命令代替） |
| `delegate_task` | ✅ 完整 | 并行子代理、深度限制、隔离上下文、子会话 |
| `execute_code` | ✅ 完整 | python3 子进程沙箱，120 秒上限 |
| `cronjob` | ✅ 完整 | create/list/update/pause/resume/remove/run；`30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划；SQLite 任务存储；调度循环 |
| `tool_search` | ✅ 完整 | 按关键词搜索已注册工具目录 |
| `vision_analyze` | ✅ 完整 | 经聊天 provider 的 `analyze_image` 路由，`[auxiliary.vision]` provider/模型覆盖 |
| `image_generate` | ✅ 完整 | OpenAI images API，PNG 存于 `<home>/images` |
| `text_to_speech` | ✅ 完整 | OpenAI TTS 或自定义 `ULNCLAW_TTS_ENDPOINT` |
| `ha_*`（4 个 Home Assistant 工具） | ✅ 完整 | Home Assistant REST API，依赖 `HASS_URL` + `HASS_TOKEN` |
| `kanban_*`（12 个工具） | ✅ 完整 | 本地 SQLite 协作看板：create/list/show/complete/block/unblock/comment/heartbeat/link/attach/attach_url/attachments |
| `browser_*`（12 个工具） | ✅ 完整 | CDP WebSocket 客户端（`browser` 模块）：端点发现、页面会话、带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框；`ULNCLAW_BROWSER_CDP` 支持 ws://、http://host:port 或 `auto`（监督器启动托管的无头 Chrome/Chromium） |
| `computer_use` | 🟡 门控 | 需要 computer-use 驱动（hermes：cua-driver） |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 门控 | 已注册，依赖平台凭据门控；后端待实现 |
| `x_search`, `video_analyze`, `video_generate`, `bfl_flux3_*` | ⬜ 暂缓 | 依赖特定供应商（xAI/BFL）；凭据就绪后再补 |

## 功能对标

| hermes 功能 | ulnclaw | 说明 |
|---|---|---|
| 工具调用代理循环 | ✅ | 迭代预算、用量统计、step 回调 |
| SQLite 状态库（`hermes_state.py`） | ✅ | sessions/messages/system_prompts/state_meta/async_delegations 表结构，FTS5（不可用时 LIKE 回退），会话血缘 |
| 会话数据库恢复（`session_recovery.py`） | ✅ 核心 | `ulnclaw sessions recover <db> [--out FILE]`：离线、非破坏性——源库连同 WAL/SHM/journal 旁车文件复制到一次性目录，规范表按列交集拷入全新当前表结构库，受损表按 rowid 逐行抢救，孤儿消息重建会话行，重建 FTS，完整性校验 + JSON 报告；绝不就地修复或覆盖在用数据库 |
| 上下文压缩（`conversation_compression.py`） | ✅ | 预算触发，中段对话经二次模型调用摘要，保留系统提示词 + 首条用户消息 + 最近尾部；摘要调用遵循 `[auxiliary.compression]` 路由 |
| 审批系统（`approval.py`） | ✅ | 命令归一化（反斜杠续行、`${IFS}`、注释剥离）、硬性底线（直接阻止）、可恢复但昂贵的操作（需确认）；REPL y/N 提示；网关运行审批（`POST /v1/runs/:id/approval`，once/session/always/deny，SSE `approval.request`）、fail-closed `[approvals] timeout`（默认 300s）、`always` 授权跨重启持久化 |
| 威胁模式扫描（`threat_patterns.py`） | ✅ 核心 | 对重新进入上下文的工具结果做提示注入扫描（建议性） |
| 工具集（`toolsets.py`） | ✅ | 全部 33 个工具集定义，含组合（`includes`），默认 `coding` |
| 工具注册表（`registry.py`） | ✅ | check_fn 门控、工具集分组、结果大小截断 |
| Provider 抽象（`runtime_provider.py`） | ✅ | OpenAI 兼容（OpenAI/OpenRouter/DashScope/Ollama/llama.cpp）、原生 Anthropic Messages 传输（`anthropic_messages`：system 参数、tool_use/tool_result 块、SSE 流式、max_tokens 上限、OAuth bearer）、本地 provider 免密钥 |
| Provider 回退链（`fallback_providers`、`try_activate_fallback`） | ✅ 核心 | `[model] fallbacks = ["provider:model", ...]`：模型调用失败时按序推进（每条目惰性构建客户端、密钥回退主运行时），激活的回退在本轮内保持生效，下一轮恢复主 provider（hermes `restore_primary_runtime`）；委派/cron 子代理继承配置 |
| 辅助模型路由（`auxiliary_client.py`） | ✅ 核心 | `[auxiliary.<task>]` 按任务覆盖 provider/模型/base_url/api_key/key_env（`compression`、`vision`）；`"auto"`/留空继承主运行时；无覆盖时复用主客户端 |
| 配置（`config.yaml`） | ✅ | `config.toml` + `.env` 文件、profiles、环境变量优先级 |
| 技能系统 | ✅ | 发现、frontmatter、关联文件 |
| 记忆系统 | ✅ | MEMORY.md/USER.md，注入提示词 |
| Cron 调度器 | ✅ | 任务存储 + 计划解析 + 轮询循环（`cron::run_scheduler`） |
| MCP 客户端（`mcp_tool.py`） | ✅ 核心 | stdio JSON-RPC：initialize/tools/list/tools/call；`[[mcp.servers]]` 配置；工具注册为 `mcp__<server>__<tool>`；npx/uvx/pipx 启动前的 OSV 恶意软件检查（`osv_check.py` 移植：MAL-* 通告阻止启动、fail-open、1 小时结论缓存、`OSV_ENDPOINT`/`OSV_CHECK_CACHE_TTL` 覆盖） |
| CLI（`hermes_cli/`） | ✅ 核心 | 带斜杠命令的聊天 REPL（含 `/rollback [N|hash] [file]`、`/rollback diff <N>`、`/diff` 检查点命令、`/recap`）、一次性 `run`、sessions/tools/skills/cron/checkpoints 子命令（含 `sessions export --format md\|html` —— SHA256 校验的 Markdown 或独立 HTML + manifest ——、`sessions recap` 与 `sessions recover`）、`moa run/list/delete`、`skills blueprints/schedule/unschedule`、`init` |
| 委派（delegation） | ✅ | SubAgentRunner trait、深度限制、子会话 |
| 混合智能体 MoA（`moa_loop.py`、`moa_config.py`） | ✅ 核心 | `[moa.presets.<name>]` 参考模型并行扇出 + 聚合器综合（`ulnclaw moa run/list/delete`、REPL `/moa <prompt>`）；loud/silent 降级策略、全部失败提前返回、聚合失败回退拼接结果；持久 `provider: moa` 门面、trace 与隐私过滤未移植 |
| HTTP 网关（`gateway/platforms/api_server.py`） | ✅ 核心 | `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions`（`X-Ulnclaw-Session-Id` 会话续接、`stream: true` SSE 令牌流 + `hermes.tool.progress` 事件）、`/v1/responses`（经 `previous_response_id` 有状态续接、`stream: true` Responses-API SSE 事件）、`/v1/models`、`/api/model/options`、`/v1/capabilities`、`/v1/runs`（异步运行 + SSE 事件 + 停止 + 审批）、`/api/sessions` 增删查改 + 会话聊天 + chat/stream + `PATCH`（title/end_reason）+ `fork` + 会话级模型锁（每轮生效）+ `recap`、`/api/jobs` 定时任务 HTTP API（增删查改 + pause/resume/run）、`/v1/skills`、`/v1/toolsets`、Bearer 令牌鉴权 |
| 消息平台（Telegram/WhatsApp/QQ 等） | ⬜ 暂缓 | hermes 的平台适配器未移植；HTTP 网关已覆盖 OpenAI 兼容前端 |
| TUI/web/app | ⬜ 暂缓 | hermes 提供 TUI 与 web 应用；ulnclaw 目前是库 + CLI + HTTP 网关 |
| 环境（`tools/environments/`） | ✅ 核心 | `terminal` 后端：local（默认）、docker（`ensure_docker_container` inspect→run）、ssh（BatchMode、identity 文件）；`[terminal] backend/container/image/ssh_host/...`；modal/daytona/vercel 暂缓 |
| 检查点管理器（`checkpoint_manager.py`） | ✅ | v2 共享 shadow git 存储（`<home>/checkpoints/store`）：按项目 ref/index，编辑前透明快照（每轮 `write_file`/`patch` 前一次），list/restore/diff/prune CLI，容量上限、超大文件过滤、孤儿/过期自动清理 |
| 浏览器监督器 | ✅ | `ULNCLAW_BROWSER_CDP=auto` 时自动启动受管 headless Chrome/Chromium |
| CUA（computer-use） | ⬜ 暂缓 | 需要 computer-use 驱动 |

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
├── sandboxes/              execute_code 脚本
├── approvals.json          持久化的 "always" 审批授权
└── checkpoints/store/      共享 shadow git 存储（按项目 ref/index）
```

## 已知差异

- CLI 上审批交互为终端 y/N 提示（hermes 有更丰富的平台化流程）；网关通过
  HTTP 暴露运行审批（once/session/always/deny）。chat-completions 请求没有
  run 上下文，确认级命令按设计自动拒绝。hermes 的 Smart-DENY（LLM 辅助）
  判定与 cron 审批模式未移植；无人值守的运行一律 fail-closed。
- 浏览器监督器直接启动本地 Chrome/Chromium；hermes 驱动外部 `agent-browser`
  守护进程（云浏览器 provider 未移植）。
- 网关实现了 api_server 平台的子集；多 profile 复用（`/p/<profile>/...`）
  未移植。任务 API 仅本地投递（`deliver="local"`）；hermes 的外部投递
  目标与 NAS/Chronos 触发 webhook（`/api/cron/fire`）未移植。
- `/api/model/options` 仅返回已配置的单个 provider 行；hermes 的多
  provider 清单（在线目录探测、定价、能力、models.dev 精选模型）未移植。
- 压缩使用 字符数/4 的 token 估算而非分词器。
- `patch` 模糊链实现了全部 9 种 hermes 策略；相似度基于 LCS 比率
  （difflib.SequenceMatcher 的等价实现），边界阈值与 CPython 实现可能略有差异。
- 环境覆盖 local/docker/ssh；hermes 的 modal/daytona/vercel 后端及其凭据
  流程未移植。
- 检查点跳过 hermes 的 pre-v2 旧存储迁移（仅全新存储），孤儿判定使用
  工作目录存在性（不记录卷设备/inode 证据）。
