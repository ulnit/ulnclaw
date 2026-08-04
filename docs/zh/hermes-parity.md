# Hermes Agent 对标矩阵 (v2026.8.3)

本文档跟踪 ulnclaw 与
[hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3)
的对应关系。ulnclaw 是 hermes agent 引擎的 Rust 重实现：相同的工具面、
相同的存储布局、相同的配置语义 —— 原生性能，单一静态二进制。

## 工具对标

| hermes 工具 | ulnclaw | 说明 |
|---|---|---|
| `terminal`, `process` | ✅ 完整 | 前台/后台执行、超时、工作目录跟踪、后台会话管理（list/log/wait/kill）、失败智能（良性退出码语义 + 输出模式恢复提示） |
| 工具输出上限（`tool_output_limits.py`） | ✅ | `[tool_output] max_bytes/max_lines/max_line_length` 可调 terminal 输出头+尾上限（默认 10 万字符）、read_file 分页上限（2000 行）与每行截断（2000 字符，`... [truncated]` 标记）；非正值回退默认；未配置时行为不变 |
| 终端失败提示（`terminal_hints.py`、`_interpret_exit_code`） | ✅ | 良性非零退出码给出 `exit_code_meaning`（grep/rg/diff/find/test/curl/git 语义表，取管道/链的最后一段，跳过 `VAR=val` 前缀）；失败命令至多附加一条 `hint`，按生产频率排序的输出模式扫描（gh JSON 字段漂移、合并冲突、命令未找到——python/pip 特判、ModuleNotFoundError/ImportError、"already exists"、gh 限流、权限拒绝）+ 退出码 124/126/137 专属提示；扫描窗口限 4000 字符，首个匹配生效 |
| 密钥脱敏（`agent/redact.py`） | ✅ 核心 | terminal 输出（前台 + process log/wait）与 read_file 内容均经脱敏器：约 55 种厂商前缀令牌（sk-/ghp_/glpat-/AKIA/xox…/JWT/私钥/数据库连接串/Authorization 与 x-api-key 头）、env 转储命令的 KEY=value 脱敏、其余场景的 JSON/YAML 密钥字段；文件读取内容使用不可复用哨兵 `«redacted:prefix…»`，避免代理把截断密钥写回（hermes #35519）；web URL 查询参数脱敏保持可选；完整日志管线与 profile 密钥作用域未移植 |
| ANSI 剥离（`ansi_strip.py`） | ✅ | 完整 ECMA-48 覆盖（CSI 含私有模式/冒号参数/中间字节、OSC 的 BEL/ST 终止、DCS/SOS/PM/APC、nF 与单字节转义、8-bit C1），terminal 与 execute_code 输出在送达模型前剥离；`sanitize_display_text` 另去除裸控制字符并归一化 CR，供终端安全回显 |
| 二进制扩展守卫（`binary_extensions.py`） | ✅ | `read_file` 以纯字符串检查（无 I/O）拒绝约 80 种二进制扩展，并提示改用 vision_analyze/terminal；`.pdf` 保持可读（文本类） |
| `read_file`, `write_file`, `patch`, `search_files` | ✅ 完整 | 带行号读取与 `next_offset` 分页、模糊替换（容忍空白/缩进差异）、V4A 多文件补丁、unified diff、ripgrep 风格搜索 |
| `web_search`, `web_extract` | ✅ 完整 | 可插拔后端：Tavily / Brave / SearXNG / 内置 DuckDuckGo；HTML→文本抽取 |
| URL 安全 / SSRF 防护（`tools/url_safety.py`） | ✅ 核心 | `url_safety` 模块：拦截对私有/内网地址的 web 拉取（环回、RFC1918、链路本地、CGNAT 100.64/10、基准 198.18/15、ULA、IPv4 映射 IPv6）；云元数据端点（169.254.169.254、metadata.google.internal、ECS 任务元数据…）**永远**拦截；接入 `web_extract`（逐 URL 检查 + 经 reqwest 重定向策略逐跳重验 + 拒绝嵌入凭证的 URL：令牌前缀与敏感查询参数拦截），可通过 `[security] allow_private_urls` / `ULNCLAW_ALLOW_PRIVATE_URLS` 放开；DNS 失败默认拦截，配置代理时委托代理解析（hermes 语义） |
| `memory` | ✅ 完整 | `MEMORY.md` + `USER.md`，原子批量 `operations`，字符上限（2200/1375），每轮注入系统提示词 |
| `todo` | ✅ 完整 | 会话任务列表、merge 模式、强制单一 `in_progress` |
| `session_search` | ✅ 完整 | SQLite FTS5 检索 + 会话内滚动，会话血缘 |
| `clarify` | ✅ 完整 | 单选/多选/开放式提问（经前端回调） |
| `skills_list`, `skill_view`, `skill_manage` | ✅ 完整 | SKILL.md frontmatter、关联文件（references/templates/scripts）、路径穿越防护 |
| 蓝图 Blueprints（`tools/blueprints.py`） | ✅ 核心 | 在 frontmatter 声明 `metadata.hermes.blueprint.schedule` 的技能可排程：`skills blueprints`（列表）、`skills schedule <name>`（创建 `blueprint:<skill>` 定时任务并挂载技能）、`skills unschedule <name>`；畸形 blueprint 块显式报错；`skills list` 以日程标注蓝图。hermes 的建议队列与 `export_blueprint` 发布路径未移植（以显式命令代替） |
| 技能守卫（`tools/skills_guard.py`） | ✅ 核心 | `skills scan <name> [--source <repo>] [--json] [--force]`：静态扫描器 `skills-guard-v1`，扫描 SKILL.md 及关联文件——119 条威胁模式（外泄/破坏/持久化/供应链/提示注入）、不可见 Unicode 检测、结构限制（50 文件 / 1 MB / 单文件 256 KB、符号链接逃逸与可执行位检查）、信任等级（builtin / agent-created / 受信任仓库含前缀别名 / community）、裁定策略（critical→dangerous、high→caution；community+caution 拦截，trusted 源遇 dangerous 同样拦截，`--force` 仅对非 community 的 caution 可覆盖） |
| `delegate_task` | ✅ 完整 | 并行子代理、深度限制、隔离上下文、子会话；hermes v2026.8.3 后台语义：顶层委派即发即忘（`mode: background`、delegation_id、`cache/delegation/live/<id>/task-N.log` 实时记录），全部子任务完成后以**单条**汇总结果重新进入会话（REPL 与网关会话聊天前置排队消费）；编排子代理（深度 > 0）保持同步；一次性/无状态会话强制同步执行并附说明（`tools/async_delegation.py` 移植，含持久 sqlite 登记：派发与汇总结果持久化于 `async_delegations`，启动恢复将崩溃后仍 `running` 的行转为终态 `unknown` 结果，drain 经持久投递认领生命周期认领未投递行——每次认领累计 `delivery_attempts`、300 秒过期认领可接管、源会话已消失的完成结果在 8 次尝试后收敛为终态 `dropped`、成功注入在认领令牌下标记 `delivered`）；`GET /v1/delegations` + `/v1/delegations/:id` 登记端点（ulnclaw 运维扩展） |
| `execute_code` | ✅ 完整 | python3 子进程沙箱，120 秒上限 |
| `cronjob` | ✅ 完整 | create/list/update/pause/resume/remove/run；`30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划；SQLite 任务存储；调度循环——网关每 30s 自动派发到期任务为受跟踪的 cron 运行（cron 审批作用域，结果回写任务行），`ulnclaw cron run <id>` 可在 CLI 立即执行一次 |
| `tool_search` | ✅ 完整 | 按关键词搜索已注册工具目录 |
| `vision_analyze` | ✅ 完整 | 经聊天 provider 的 `analyze_image` 路由，`[auxiliary.vision]` provider/模型覆盖 |
| `image_generate` | ✅ 完整 | OpenAI images API，PNG 存于 `<home>/images` |
| `text_to_speech` | ✅ 完整 | OpenAI TTS 或自定义 `ULNCLAW_TTS_ENDPOINT` |
| `ha_*`（4 个 Home Assistant 工具） | ✅ 完整 | Home Assistant REST API，依赖 `HASS_URL` + `HASS_TOKEN` |
| `kanban_*`（12 个工具） | ✅ 完整 | 本地 SQLite 协作看板：create/list/show/complete/block/unblock/comment/heartbeat/link/attach/attach_url/attachments |
| `browser_*`（12 个工具） | ✅ 完整 | CDP WebSocket 客户端（`browser` 模块）：端点发现、页面会话、带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框；`ULNCLAW_BROWSER_CDP` 支持 ws://、http://host:port 或 `auto`（监督器启动托管的无头 Chrome/Chromium）；已移植 hermes SSRF 防护（`browser/guard.rs`）：敏感查询参数 + 云元数据底线无条件拦截，非本地端点或容器化终端启用私网地址防护，重定向落地复检，console/eval 表达式内 URL 字面量预筛，私有页面下原始 CDP 方法白名单；浏览器输出强制脱敏；REPL `/browser connect` 与网关 `/v1/browser/connect|disconnect|status` 实时切换端点；`CAMOFOX_URL` 接入 Camofox REST 后端（其他云浏览器 provider 仍未移植） |
| `close_terminal`、`read_terminal`、`focus_pane`、`open_preview` | ✅ 核心 | 桌面 GUI 工具（hermes `close_terminal_tool.py` / `read_terminal_tool.py` / `focus_pane_tool.py` / `open_preview_tool.py`）：仅在 `ULNCLAW_DESKTOP=1` 下注册，经 `desktop` 桥接层路由——宿主应用安装事件发射器（`ulnclaw::desktop::set_emitter`）接收 `(ui_session_id, event, payload)` 事件（`terminal.close`、`pane.reveal`、`preview.open`）及阻塞式 `read_terminal` 回调；未接入宿主时返回 "desktop only"，从不杀进程，并规范化裸域名（`www.cnn.com` → https、`localhost:3000` → http）；`react_to_message`（hermes `react_to_message_tool.py` 移植）：代理表情回应——每作者一个、重发相同表情即撤回，默认最新用户消息（`messages_back` 回溯、`message_row_id` 精确指定），持久于 `messages.display_metadata` 并经 `message.reaction` 桥接事件实时渲染；门控于 `ULNCLAW_DESKTOP=1` **与** `[display] message_reactions` |
| `computer_use` | 🟡 门控 | 需要 computer-use 驱动（hermes：cua-driver） |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 门控 | 已注册，依赖平台凭据门控；后端待实现 |
| `x_search` | 🟡 门控 | hermes `x_search_tool.py` 完整移植：xAI Responses-API `x_search` 服务端工具，支持账号白/黑名单（最多 10 个、去 `@`）、严格客户端日期范围校验（YYYY-MM-DD、禁止倒置/纯未来窗口）、`enable_image_understanding` / `enable_video_understanding`、5xx/瞬时错误退避重试、过滤无引文时的 `degraded`/`degraded_reason` 标记、`[x_search]` 配置（model / reasoning_effort / timeout_seconds / retries）；仅在 `XAI_API_KEY` 存在**且**启用可选 `x_search` 工具集时注册（hermes 对齐——SuperGrok OAuth 路径未移植） |
| `video_analyze` | ✅ 核心 | hermes `vision_tools.video_analyze_tool` 完整移植：本地文件 / `file://` / HTTP(S) 来源（远程下载经 SSRF 防护，缓存于 `cache/video/temp_video_files/` 并自动清理）、扩展名→mime 映射（mp4/webm/mov/avi/mkv/mpeg/mpg）、20 MB 警告 + 50 MB base64 硬上限、内联 `video_url` data-URL 载荷、`[auxiliary.vision]` 路由失败回落主 provider、空响应重试一次；可选 `video` 工具集（hermes 对齐）——需要支持视频的 provider |
| `video_generate`、`bfl_flux3_*` | ⭜ 暂缓 | provider 插件注册表（xAI/BFL/Pixverse/…）——BFL 工具经 Nous 网关运行；凭据就绪后再补 |

## 功能对标

| hermes 功能 | ulnclaw | 说明 |
|---|---|---|
| 工具调用代理循环 | ✅ | 迭代预算、用量统计、step 回调 |
| SQLite 状态库（`hermes_state.py`） | ✅ | sessions/messages/system_prompts/state_meta/async_delegations 表结构，FTS5（不可用时 LIKE 回退），会话血缘 |
| 会话数据库恢复（`session_recovery.py`） | ✅ 核心 | `ulnclaw sessions recover <db> [--out FILE]`：离线、非破坏性——源库连同 WAL/SHM/journal 旁车文件复制到一次性目录，规范表按列交集拷入全新当前表结构库，受损表按 rowid 逐行抢救，孤儿消息重建会话行，重建 FTS，完整性校验 + JSON 报告；绝不就地修复或覆盖在用数据库 |
| 环境探针（`tools/env_probe.py`） | ✅ | 终端后端为本地时，向系统提示注入一行确定性的 Python 工具链说明：python3/python 版本、pip 模块可用性、`pip`↔`python3` 版本错配、PEP 668 外部管理标记（有 uv 时不告警）；健康环境保持静默；进程级缓存由单一后台线程构建，调用方最多等 10 秒后放行；远端后端（docker/ssh）跳过探测；`[agent] environment_probe` 开关（默认开启） |
| 上下文压缩（`conversation_compression.py`） | ✅ | 预算触发，中段对话经二次模型调用摘要，保留系统提示词 + 首条用户消息 + 最近尾部；摘要调用遵循 `[auxiliary.compression]` 路由 |
| 审批系统（`approval.py`） | ✅ | 命令归一化（反斜杠续行、`${IFS}`、注释剥离）、硬性底线（直接阻止）、可恢复但昂贵的操作（需确认）；REPL y/N 提示；网关运行审批（`POST /v1/runs/:id/approval`，once/session/always/deny，SSE `approval.request`）、fail-closed `[approvals] timeout`（默认 300s）、`always` 授权跨重启持久化；`[approvals] mode = manual|smart|off` —— smart 模式先询问辅助守护 LLM（防提示注入的提示词设计，运维 `smart_policy` 仅走可信通道），不确定时升级人工，`off` 在硬性底线以下自动放行；`cron_mode = deny|approve` 管控无人值守 cron 运行（deny = fail-closed 默认） |
| 威胁模式扫描（`threat_patterns.py`） | ✅ 核心 | 对重新进入上下文的工具结果做提示注入扫描（建议性） |
| 工具集（`toolsets.py`） | ✅ | 全部 33 个工具集定义，含组合（`includes`），默认 `coding` |
| 工具注册表（`registry.py`） | ✅ | check_fn 门控、工具集分组、结果大小截断 |
| Provider 抽象（`runtime_provider.py`） | ✅ | OpenAI 兼容（OpenAI/OpenRouter/DashScope/Ollama/llama.cpp）、原生 Anthropic Messages 传输（`anthropic_messages`：system 参数、tool_use/tool_result 块、SSE 流式、max_tokens 上限、OAuth bearer）、本地 provider 免密钥 |
| Provider 回退链（`fallback_providers`、`try_activate_fallback`） | ✅ 核心 | `[model] fallbacks = ["provider:model", ...]`：模型调用失败时按序推进（每条目惰性构建客户端、密钥回退主运行时），激活的回退在本轮内保持生效，下一轮恢复主 provider（hermes `restore_primary_runtime`）；委派/cron 子代理继承配置 |
| 辅助模型路由（`auxiliary_client.py`） | ✅ 核心 | `[auxiliary.<task>]` 按任务覆盖 provider/模型/base_url/api_key/key_env（`compression`、`vision`）；`"auto"`/留空继承主运行时；无覆盖时复用主客户端 |
| models.dev 目录（`agent/models_dev.py`） | ✅ 核心 | `models_dev.rs`：拉取 `https://models.dev/api.json`，三级缓存——内存（1 小时 TTL，过期数据立即返回并由后台线程刷新）→ 磁盘（`$ULNCLAW_HOME/models_dev_cache.json`，任意陈旧度可用）→ 网络单飞获取（失败后进程级退避 5 分钟）；provider ID 映射 + 同名回退、上下文/能力查询（大小写不敏感、`:cloud`/`-cloud` 后缀回退）、agentic 目录过滤（噪声模式 + Google 隐藏清单）、`get_provider_info`/`get_model_info`；`ULNCLAW_MODELS_DEV_URL` 镜像覆盖（http(s)/file）、`ULNCLAW_MODELS_DEV_CACHE` 路径覆盖；网关 `/api/model/options` 目录增强 + `?refresh=true`；CLI `ulnclaw models providers\|list\|info\|refresh` |
| 配置（`config.yaml`） | ✅ | `config.toml` + `.env` 文件、profiles、环境变量优先级 |
| 技能系统 | ✅ | 发现、frontmatter、关联文件 |
| 记忆系统 | ✅ | MEMORY.md/USER.md，注入提示词 |
| Cron 调度器 | ✅ | 任务存储 + 计划解析 + 轮询循环（`cron::run_scheduler`） |
| MCP 客户端（`mcp_tool.py`） | ✅ 核心 | stdio JSON-RPC：initialize/tools/list/tools/call；`[[mcp.servers]]` 配置；工具注册为 `mcp__<server>__<tool>`；npx/uvx/pipx 启动前的 OSV 恶意软件检查（`osv_check.py` 移植：MAL-* 通告阻止启动、fail-open、1 小时结论缓存、`OSV_ENDPOINT`/`OSV_CHECK_CACHE_TTL` 覆盖） |
| CLI（`hermes_cli/`） | ✅ 核心 | 带斜杠命令的聊天 REPL（含 `/rollback [N|hash] [file]`、`/rollback diff <N>`、`/diff` 检查点命令、`/recap`）、一次性 `run`、sessions/tools/skills/cron/checkpoints 子命令（含 `sessions export --format md\|html` —— SHA256 校验的 Markdown 或独立 HTML + manifest ——、`sessions recap` 与 `sessions recover`）、`moa run/list/delete`、`models providers/list/info/refresh`（models.dev 目录）、`skills blueprints/schedule/unschedule`、`diff`、`init` |
| Git 工作区 diff（`working_diff.py`） | ✅ | `ulnclaw diff [--staged|--all] [--dir PATH] [paths...]` + REPL `/gitdiff [staged|all]`：working/staged/all 三模式，未跟踪文件经 `git diff --no-index` 折入（上限 50 个），带超时；基于检查点的 REPL `/diff` 保持独立 |
| 委派（delegation） | ✅ | SubAgentRunner trait、深度限制、子会话 |
| 混合智能体 MoA（`moa_loop.py`、`moa_config.py`） | ✅ 核心 | `[moa.presets.<name>]` 参考模型并行扇出 + 聚合器综合（`ulnclaw moa run/list/delete`、REPL `/moa <prompt>`）；loud/silent 降级策略、全部失败提前返回、聚合失败回退拼接结果；持久 `provider: moa` 门面、trace 与隐私过滤未移植 |
| HTTP 网关（`gateway/platforms/api_server.py`） | ✅ 核心 | `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions`（`X-Ulnclaw-Session-Id` 会话续接、`stream: true` SSE 令牌流 + `hermes.tool.progress` 事件）、`/v1/responses`（经 `previous_response_id` 有状态续接、`stream: true` Responses-API SSE 事件）、`/v1/models`、`/api/model/options`（models.dev 目录增强、`?refresh=true`）、`/v1/capabilities`、`/v1/runs`（异步运行 + SSE 事件 + 停止 + 审批）、`/api/sessions` 增删查改 + 会话聊天 + chat/stream + `PATCH`（title/end_reason）+ `fork` + 会话级模型锁（每轮生效）+ `recap`、`/api/jobs` 定时任务 HTTP API（增删查改 + pause/resume/run）、`/v1/skills`、`/v1/toolsets`、`/metrics`（Prometheus 计数器/量表——ulnclaw 运维扩展）、`/api/usage`（令牌核算：进程计数器 + 全时会话库总量 + 按会话明细——ulnclaw 运维扩展）、`/v1/delegations`（后台委派登记——ulnclaw 运维扩展）、`/v1/browser/status|connect|disconnect`（实时 CDP 端点控制，对齐 hermes `/browser connect`——ulnclaw 运维扩展）、Bearer 令牌鉴权 |
| 消息平台（Telegram/WhatsApp/QQ 等） | ⬜ 暂缓 | hermes 的平台适配器未移植；HTTP 网关已覆盖 OpenAI 兼容前端 |
| TUI/web/app | ⬜ 暂缓 | hermes 提供 TUI 与 web 应用；ulnclaw 目前是库 + CLI + HTTP 网关——`desktop` 桥接层是 GUI 宿主安装事件发射器的嵌入接缝 |
| 沙箱环境清洗 + passthrough（`environments/local.py` 黑名单、`env_passthrough.py`） | ✅ | terminal/execute_code 子进程继承的环境会剔除 provider/工具凭证黑名单与虚拟环境标记（`VIRTUAL_ENV`/`CONDA_PREFIX`）；技能 `required_environment_variables`（`skill_view` 时注册）与 `[terminal] env_passthrough` 放行其余变量——受保护的 provider 凭证与 `AUXILIARY_*_API_KEY`/`GATEWAY_RELAY_*` 动态密钥永远被拒绝（hermes GHSA-rhgp-j443-p4rf，失败即关闭） |
| 环境（`tools/environments/`） | ✅ 核心 | `terminal` 后端：local（默认）、docker（`ensure_docker_container` inspect→run）、ssh（BatchMode、identity 文件）；`[terminal] backend/container/image/ssh_host/...`；modal/daytona/vercel 暂缓 |
| 检查点管理器（`checkpoint_manager.py`） | ✅ | v2 共享 shadow git 存储（`<home>/checkpoints/store`）：按项目 ref/index，编辑前透明快照（每轮 `write_file`/`patch` 前一次），list/restore/diff/prune CLI，容量上限、超大文件过滤、孤儿/过期自动清理 |
| 浏览器监督器 | ✅ | `ULNCLAW_BROWSER_CDP=auto` 时自动启动受管 headless Chrome/Chromium |
| Camofox 后端（`tools/browser_camofox.py`） | ✅ 核心 | `browser/camofox.rs`：`CAMOFOX_URL` REST 反检测浏览器（Camoufox）后端——全部 12 个 browser 工具经 REST 路由（标签页会话、带元素引用的可访问性快照、点击/输入/滚动/后退/按键、从快照提取图片、截图供视觉分析）；CDP 覆盖优先；`CAMOFOX_API_KEY` bearer 鉴权、`CAMOFOX_USER_ID`/`CAMOFOX_SESSION_KEY` 身份覆盖 + 已有标签页收养、Docker 环回 URL 重写（`CAMOFOX_REWRITE_LOOPBACK_URLS` + 别名）、从 `/health` 发现 VNC URL、读取操作的 SSRF 私有页面防护、console/原始 CDP/对话框明确报不支持；`CAMOFOX_MANAGED_PERSISTENCE` 受管持久化（稳定的 UUIDv5 profile 级 userId，对应 hermes `browser.camofox.managed_persistence`）；网关与 REPL browser status 报告后端 |
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
  run 上下文，确认级命令按设计自动拒绝。Smart 审批（LLM 守护）与 cron
  审批模式已移植；无人值守的运行默认 fail-closed，`cron_mode = "approve"`
  可放行。
- 浏览器监督器直接启动本地 Chrome/Chromium；hermes 驱动外部 `agent-browser`
  守护进程。Camofox REST 后端已移植（含受管持久化，以
  `CAMOFOX_MANAGED_PERSISTENCE` 环境变量替代 hermes 的 config.yaml 开关）；
  其他云浏览器 provider 未移植。
- 网关实现了 api_server 平台的子集；多 profile 复用（`/p/<profile>/...`）
  未移植。任务 API 仅本地投递（`deliver="local"`）；hermes 的外部投递
  目标与 NAS/Chronos 触发 webhook（`/api/cron/fire`）未移植。
- `/api/model/options` 以 models.dev 目录增强已配置的单个 provider 行
  （模型清单 + 能力/成本映射、`?refresh=true`）；hermes 的多 provider
  选择器清单（多 provider 探测、精选模型、凭据池行）未移植。
- 压缩使用 字符数/4 的 token 估算而非分词器。
- `patch` 模糊链实现了全部 9 种 hermes 策略；相似度基于 LCS 比率
  （difflib.SequenceMatcher 的等价实现），边界阈值与 CPython 实现可能略有差异。
- 环境覆盖 local/docker/ssh；hermes 的 modal/daytona/vercel 后端及其凭据
  流程未移植。
- 检查点跳过 hermes 的 pre-v2 旧存储迁移（仅全新存储），孤儿判定使用
  工作目录存在性（不记录卷设备/inode 证据）。
