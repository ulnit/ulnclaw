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
| `kanban_*`（12 个工具） | ✅ 完整 | 本地 SQLite 协作看板，与 `ulnclaw kanban` CLI、网关 `/api/kanban/*` 端点共用同一 `KanbanStore` 引擎与 `kanban.db`（一块看板、三个界面 —— hermes 对齐）：create（支持 `parents`）/list/show/comment/heartbeat（自动认领 todo→ready→running）/complete/block/unblock/link/attach/attach_url/attachments；唯一前缀 id 解析，`ULNCLAW_KANBAN_TASK`/`HERMES_KANBAN_TASK` 工作进程上下文（worker 省略 task_id 默认自身任务；create/unblock/link 仅限编排者，hermes 门控语义），REPL `/kanban` 看板操作经 `run_slash` |
| `browser_*`（12 个工具） | ✅ 完整 | CDP WebSocket 客户端（`browser` 模块）：端点发现、页面会话、带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框；`ULNCLAW_BROWSER_CDP` 支持 ws://、http://host:port 或 `auto`（监督器启动托管的无头 Chrome/Chromium）；已移植 hermes SSRF 防护（`browser/guard.rs`）：敏感查询参数 + 云元数据底线无条件拦截，非本地端点或容器化终端启用私网地址防护，重定向落地复检，console/eval 表达式内 URL 字面量预筛，私有页面下原始 CDP 方法白名单；浏览器输出强制脱敏；REPL `/browser connect` 与网关 `/v1/browser/connect|disconnect|status` 实时切换端点；`CAMOFOX_URL` 接入 Camofox REST 后端（其他云浏览器 provider 仍未移植） |
| `close_terminal`、`read_terminal`、`focus_pane`、`open_preview` | ✅ 核心 | 桌面 GUI 工具（hermes `close_terminal_tool.py` / `read_terminal_tool.py` / `focus_pane_tool.py` / `open_preview_tool.py`）：仅在 `ULNCLAW_DESKTOP=1` 下注册，经 `desktop` 桥接层路由——宿主应用安装事件发射器（`ulnclaw::desktop::set_emitter`）接收 `(ui_session_id, event, payload)` 事件（`terminal.close`、`pane.reveal`、`preview.open`）及阻塞式 `read_terminal` 回调；未接入宿主时返回 "desktop only"，从不杀进程，并规范化裸域名（`www.cnn.com` → https、`localhost:3000` → http）；`react_to_message`（hermes `react_to_message_tool.py` 移植）：代理表情回应——每作者一个、重发相同表情即撤回，默认最新用户消息（`messages_back` 回溯、`message_row_id` 精确指定），持久于 `messages.display_metadata` 并经 `message.reaction` 桥接事件实时渲染；门控于 `ULNCLAW_DESKTOP=1` **与** `[display] message_reactions` |
| `computer_use` | ✅ 核心 | cua-driver MCP 后端（`src/computer_use.rs`）—— 完整 hermes 工具 schema + 审批语义，见下方 Computer Use 行；驱动可达即注册（`ulnclaw computer-use doctor`） |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 门控 | 已注册，依赖平台凭据门控；后端待实现 |
| `x_search` | 🟡 门控 | hermes `x_search_tool.py` 完整移植：xAI Responses-API `x_search` 服务端工具，支持账号白/黑名单（最多 10 个、去 `@`）、严格客户端日期范围校验（YYYY-MM-DD、禁止倒置/纯未来窗口）、`enable_image_understanding` / `enable_video_understanding`、5xx/瞬时错误退避重试、过滤无引文时的 `degraded`/`degraded_reason` 标记、`[x_search]` 配置（model / reasoning_effort / timeout_seconds / retries）；仅在 `XAI_API_KEY` 存在**且**启用可选 `x_search` 工具集时注册（hermes 对齐——SuperGrok OAuth 路径未移植） |
| `video_analyze` | ✅ 核心 | hermes `vision_tools.video_analyze_tool` 完整移植：本地文件 / `file://` / HTTP(S) 来源（远程下载经 SSRF 防护，缓存于 `cache/video/temp_video_files/` 并自动清理）、扩展名→mime 映射（mp4/webm/mov/avi/mkv/mpeg/mpg）、20 MB 警告 + 50 MB base64 硬上限、内联 `video_url` data-URL 载荷、`[auxiliary.vision]` 路由失败回落主 provider、空响应重试一次；可选 `video` 工具集（hermes 对齐）——需要支持视频的 provider |
| `video_generate`、`bfl_flux3_*` | ✅ 核心 | `video_gen.rs` provider 注册表（hermes 插件设计：单一可用后端自动选中、配置名 fail-closed、`success_response`/`error_response` 契约）+ 统一 `video_generate` 工具（文生视频/图生视频/参考图生成、软校验、模型解析顺序 参数 > `[video_gen]` 配置 > provider 默认）；`managed_gateway.rs` Nous 工具网关传输（auth.json bearer + `TOOL_GATEWAY_USER_TOKEN`、`{vendor}-gateway` URL 构造、预签名 `nous-upload:` 媒体上传）；6 个 `bfl_flux3_*` 工具带固定 schema、本地路径上传预处理、轮询至完成（限流/传输错误处理、240s 兜底）、签名 URL 下载到 `~/Downloads`（`.part` 暂存 + 冲突后缀）+ 提示词指南；`video_gen_xai.rs` xAI Imagine 后端（auth.json OAuth access-token 复用 → `XAI_API_KEY` 回退、文生/图生视频模型路由含 1.5 模型、edit/extend 提交+轮询流程）+ `xai_video_edit`/`xai_video_extend` 工具（公网 HTTPS URL 校验、`provider_not_configured` 门控）；`video_gen_backends.rs` FAL 后端（六大模型家族 —— LTX 2.3、Pixverse v6、Veo 3.1、Seedance 2.0、Kling v3 4K、Happy Horse —— 能力驱动载荷、`FAL_KEY` 直连队列 REST 或 Nous `fal-queue` 托管网关）与 DeepInfra 后端（OpenAI 兼容 `/videos` 创建→轮询→下载到 `~/videos`）；不做 OAuth 刷新 —— 缓存的 Nous token 原样使用 |
| `project_list`、`project_create`、`project_switch` | ✅ 核心 | hermes `tools/project_tools.py` + `hermes_cli/projects_db.py` 完整移植：每 profile `projects.db`（projects / project_folders / project_meta / discovered_repos，WAL + DELETE 回退 + 增量列迁移）、slug 校验 + `-2` 冲突后缀、多文件夹工作区与主目录指针（首个文件夹隐式为主、删除时降级/重指）、归档/恢复/硬删除（文件夹级联）、活动项目指针、最长前缀 `project_for_path` 解析、确定性 kanban 分支名（`<slug>/<task-id>[-<title-slug>]`）、带策略协调的仓库发现缓存；工具置于可选 `project` 工具集（仅 GUI 会话 —— 与 hermes 一致不进核心集），宿主应用可安装工作区重锚回调（`projects_db::set_project_workspace_callback`） |
| 技能使用遥测 + 学习图谱（`skill_usage`、`learning_graph`、`learning_mutations`） | ✅ 核心 | hermes `tools/skill_usage.py` + `agent/learning_graph.py` + `agent/learning_mutations.py` 移植：`<home>/skills/.usage.json` 旁路记录（view/use/patch 计数、生命周期状态、固定、agent 创建溯源、原子写入），遥测接入 `skill_view`/`skill_manage`（bump view/patch、标记 agent 创建、删除时 forget），经 `skills/.archive` 的技能归档/恢复（冲突时间戳后缀、固定技能拒绝）；学习图谱载荷 —— 已学技能过滤（agent 创建或使用过）、`related_skills` 边、`MEMORY.md`/`USER.md` 条目记忆卡片、词法 记忆→技能 边（每卡 top-4）、聚类 + 密度统计；journey 节点变更（`node_detail`/`delete_node`/`edit_node`）与 memory 工具的条目格式对齐 |
| 学习时间线 / `journey` CLI（`learning_graph_render`、`journey`） | ✅ 核心 | hermes `agent/learning_graph_render.py` + `hermes_cli/journey.py` 移植：`learning_graph_render.rs` —— 桌面同源色彩数学（调色板推导、互补记忆色、smoothstep 年龄渐变）、新旧度计算（带时间 + 序号回退）、日/月/年分桶时间线（按主导类别着色的技能/记忆比例条 —— 学习热图）、编号 charted-signal 标记、累计轨迹 sparkline、图例/坐标轴/摘要装饰；`ulnclaw journey` CLI —— 时间线帧（`--reveal`、`--width/--height`、`--no-color`）、`--play` 动画、`--json` 载荷导出、`journey list`、`journey delete <node> [-y]`（技能归档、记忆重写）、`journey edit <node>` 经 `$EDITOR`；TUI 预渲染（`render_frames`）与 GUI 星图仍为桌面专属 |
| 技能策展 CLI（`curator`） | ✅ 核心 | hermes `hermes_cli/curator.py` 本地半区（LLM 整合运行留在桌面侧）：`curator.rs` —— 空闲天数计算（活动优先、created_at 回退）、裁剪候选选择（agent 创建、未固定、未归档、空闲 ≥ N 天、最空闲优先）、状态汇总、相对时间渲染；`skill_usage.rs` 报表 —— `usage_report`（磁盘上全部技能含溯源/计数/最近活动）、`unmanaged_report` / `list_unmanaged_skill_names` / `adopt_skill`（溯源标记）、`list_archived_skill_names`；CLI `ulnclaw curator status\|pin\|unpin\|archive\|restore\|list-archived\|usage [--sort activity\|name\|recent] [--json]\|prune [--days N] [--dry-run] [-y]\|adopt [names \| --all-unmanaged] [--dry-run] [-y]\|list-unmanaged`；同时以进程级 env 锁加固网关 env 覆盖测试 |
| 持久化目标 / Ralph 循环（`goals`） | ✅ 核心 | hermes `hermes_cli/goals.py` 移植：`goals.rs` —— `GoalContract`（outcome/verification/constraints/boundaries/stop_when，别名表 `parse_contract` 使无关冒号不被误解析，空字段省略，带标签 `render_block`）、`GoalState` serde 往返（状态、轮次预算、子目标、解析/传输失败计数、pid/会话/时间等待屏障）、`parse_judge_response`（verdict + 旧式 `done` 布尔、去代码围栏、内嵌 JSON 提取、无目标时 wait 指令降级）、面向裁判的背景进程块渲染；`GoalManager` 按会话编排并持久化于 `state_meta`（键 `goal:<session_id>`，set/set_contract/pause/resume/clear/mark_done，子目标增删清，wait_on/wait_on_session/wait_for_seconds/stop_waiting 惰性自动清除，status_line，contract>subgoals>plain 优先级的 next_continuation_prompt，render_contract）；fail-open `judge_goal` 经 `goal_judge` 辅助任务（contract>subgoals>plain 提示、背景进程、传输/解析失败追踪）+ `draft_contract`；`evaluate_after_turn` 状态机拆分为可纯测的 `apply_verdict`（等待屏障短路不耗轮次、WAIT 停泊、DONE、传输连续 5 次自动暂停、解析连续 3 次自动暂停、轮次预算耗尽、continue）+ 异步裁判包装；`migrate_goal_to_session`；terminal.rs 新增后台进程 pid 捕获 + `background_process_running`/`background_process_exists`/`list_background_processes` 支撑会话等待屏障；REPL `/goal`（status/show/draft/pause/resume/clear/wait/unwait、内联契约、自动启动）+ `/subgoal`（list/add/remove/clear）；`AuxiliaryTaskConfig.max_tokens` 配置项 |
| 网关 profile 多路复用（`/p/<profile>`）+ CDP 会话存活 | ✅ 核心 | hermes api_server profile 前缀中间件移植：所有网关路由镜像到 `/p/<profile>/...`；`[gateway] multiplex_profiles = true` 时每个镜像由独立栈支撑（agent 取自 `[profiles.<name>]` 覆盖，home 按 profile 隔离 `<home>/profiles/<name>` —— state.db/approvals.json/cron/skills），惰性构建并缓存（`ProfileHub`），未知 profile → 404 `Unknown or unconfigured profile`；多路复用关闭时前缀被接受但由默认 profile 服务（对齐 hermes `_resolve_request_profile`）；镜像同样经 bearer 鉴权。CDP 客户端加固：`CdpClient.is_connected`（读/写循环在套接字断开时翻转 closed 标志并让在途调用快速失败 —— 不再空等 30 秒超时），`with_session` 透明丢弃已死的缓存会话并重建 |
| 启动提示（`tips.py`） | ✅ 核心 | `tips.rs`：面向 ulnclaw 自身功能面重写的特性发现一句话语料（斜杠命令、目标、CLI 子命令、配置项、工具、网关、隐藏技巧）+ 无依赖 xorshift64* `get_random_tip`；聊天 REPL 在启动与 `/new` 时打印 `✦ Tip:` 行（对齐 hermes 欢迎/新会话提示） |
| REPL 显示与输入体验（`hermes_cli/focus_view.py`、`prompt_stash.py`、`clipboard.py`） | ✅ 核心 | `src/focus_view.rs`、`src/prompt_stash.rs`、`src/clipboard.rs` —— 三个 hermes CLI 体验模块。**专注视图**（`/focus [on\|off\|status]`）：纯显示层的精简输出模式 —— 开启时把工具进度吸附为 `off` 并记住用户原模式（`/focus off` 原样恢复），按轮诚实统计被隐藏的工具行（只计配置模式本会显示的行），轮末打印 `⋯ N tool lines hidden · /focus off to show` 恢复提示，另提供 `◉ focus` 状态栏段；纯显示不变式：绝不改变发往模型的任何字节。**工具进度**（`/verbose [off\|new\|all\|verbose]`）：REPL 工具回调滚动行（`⚙ <tool>` 行；`new` 去重连续同名）上的 hermes tool_progress_mode 档位循环。**草稿暂存**（`/stash [text\|list\|pop [n]\|drop <n>\|clear]`）：会话级纯内存草稿栈（hermes Ctrl+S 手势：有内容→暂存、空输入+1 条→弹回、空输入+多条→浏览；新者在前、上限 20 条、60 字符预览、`📌 n` 提示符指示、绝不落盘）。**剪贴板**（`/paste`）：跨平台剪贴板图片提取，以 PNG 存至 `<home>/clipboard/`（macOS pngpaste/osascript、Windows/WSL2 PowerShell WinForms + Get-Clipboard + FileDropList 回退、Linux Wayland wl-paste 非 PNG 经 ImageMagick 归一化、X11 xclip）+ `write_clipboard_text`（pbcopy → Set-Clipboard base64 → wl-copy → xclip → xsel，CJK 安全）+ SSH 会话检测（OSC 52 提示）；桌面端 Ctrl+S 键绑定保留在 Tauri 壳层 |
| 会话裁剪/归档/统计（`session_filters.py`） | ✅ 核心 | `session/filters.rs` —— 时长解析（`5h`/`30m`/`2d`/`1w`，裸数字 = 天）、时间点解析（时长 = 距今多久前；ISO 时间戳 naive=本地时区）、epoch 格式化、`PruneFilters` 类型化 WHERE 子句构建器（仅限已结束、last_active 取 COALESCE(MAX(消息时间), started_at)、source/end_reason 精确匹配、title/model 大小写不敏感子串、cwd 前缀、消息/令牌/工具调用上下界、三态 archived）+ 可读 `describe()`；存储层 `list_prune_candidates`（按最旧活动排序）、`prune_sessions`（先删消息 + FTS）、`archive_sessions`（软隐藏、幂等）、`set_session_archived`、`session_count_by_source`；CLI `ulnclaw sessions prune|archive`（hermes 语义：裸 prune = 90 天以上，任一过滤器抑制隐式截断，裸 archive 拒绝执行，预览 + y/N 确认 + `--dry-run`、`--include-archived`）与 `sessions stats`（总量、按源计数、库大小）；hermes 的计费/聊天/分支/成本过滤器对应 ulnclaw 未跟踪的列，不移植 |
| 皮肤/主题引擎（`skin_engine.py`） | ✅ 核心 | `skin.rs`：hermes 全部 9 个内置皮肤以数据形式内置（default、ares、mono、slate、daylight、warm-lightmode、poseidon、sisyphus、charizard —— 258 个颜色条目 + 品牌文案 + spinner 表情），局部调色板向 default 皮肤继承（`build_skin_config`），`list_skins`/`load_skin`（未知 → default），进程级活动皮肤（`init_skin_from_config` 读取 `[display] skin`，`get/set_active_skin`），`get_color`/`get_branding` 访问器，真彩 ANSI `colorize`（遵循 NO_COLOR）；`ulnclaw skins` CLI 列出主题并标记当前激活；REPL 提示行以活动皮肤的 `banner_dim` 着色。延后：`<home>/skins/` 用户 YAML 皮肤（无 YAML 依赖）、TUI 状态栏/prompt-toolkit 表面 |
| 欢迎横幅与更新检查（`banner.py`） | ✅ 核心 | `banner.rs`：皮肤着色的欢迎面板（盒线绘制）—— 盲文爪痕主视觉 + 模型行（缩短名称、去 `.gguf`、28 字符上限、models.dev 上下文经 `spawn_blocking` + 2 秒上限查询）、`approvals.mode = "off"` 警告（hermes YOLO 行）、cwd + 会话 id、"Available Tools" 按启用工具集分组（显示 8 个，`+N more toolsets`）、技能按类别 + `+N more` 溢出、`N tools · N skills · /help for commands` 汇总行；≥95 列终端额外显示 ULNCLAW 块字标（与 hermes 门限一致）；git 更新检查 6 小时缓存于 `$ULNCLAW_HOME/.update_check`（版本变化即失效）—— 作用域 `git fetch` 落后计数 + 浅克隆 SHA 对比路径，官方 SSH 远端走 `git ls-remote`（计数未知 → `-1` 哨兵），仓库目录 = `$ULNCLAW_REPO` → 构建期 `CARGO_MANIFEST_DIR` → `$ULNCLAW_HOME/ulnclaw`；agent 构建期间后台线程 `prefetch_update_check` + `get_update_result(500ms)`；面板标题版本标签 `ulnclaw vX · upstream <sha8>`（+carried commits），最新 tag 查询 + gitee 发布 URL（进程级缓存）。延后：标题富链接、皮肤 `banner_hero`/`banner_logo` 覆盖 |
| 浏览器 CDP 接入层（`browser_connect.py`） | ✅ 核心 | `browser/connect.rs`：Chromium 系候选发现（macOS/Windows/Linux，含 WSL `/mnt/c` 安装路径），覆盖 Chrome/Chromium/Brave/Edge；双栈回环 CDP 探测 —— `is_browser_debug_ready`（`/json/version` → `/json`，`ws://…/devtools/browser/…` 走 TCP 连通）、`discover_local_cdp_url`（先 IPv4 后 `[::1]`，捕获被 IPv4 占用者挤到纯 IPv6 的浏览器）；端口仲裁 —— `local_port_in_use` 区分空闲与被占用，`find_free_debug_port` 要求双栈回环均可绑定；带诊断的可视调试浏览器启动 `launch_chrome_debug`（逐候选 `LaunchAttempt`：ready/starting/exited/spawn-failed，stderr 尾部写入 `<home>/chrome-debug/launch-stderr.log`，退出码 0 的单实例吸收提示，`manual_chrome_debug_command` 兜底含 macOS `open -a` 形式）；`connect_local_default` 组合出 hermes `/browser connect` 完整默认流程。REPL 裸 `/browser connect` 执行该流程，成功后设置实时覆盖并向会话注入 hermes 系统备注；`/browser disconnect` 注入回退备注。托管启动候选表同步补齐 Brave/Edge。网关 `/v1/browser/*` 保持不变（已对齐） |
| Doctor（`doctor.py`） | ✅ 核心 | `doctor.rs` + `ulnclaw doctor` CLI：hermes 盒线横幅报告，✓/⚠/✗/ℹ 分级检查按段组织 —— Version & Updates（P61 的 git 状态 + 6 小时缓存落后计数）、Configuration Files（config.toml 存在性/TOML 合法性/模型已配置、`.env` 密钥扫描）、Directory Structure（home + sessions/skills/memory/cron/checkpoints/logs、state.db）、Auth Providers（`resolve_api_key` 链：config → ULNCLAW_API_KEY → OPENAI_API_KEY → ANTHROPIC_API_KEY；本地免密钥提供商单独提示）、External Tools（git、P62 的 Chromium 系候选、内置 SQLite）、Toolsets（启用/禁用 + 经 `resolve_toolset` 检测未知名称）、Skills（安装数量 + frontmatter 健全性）、Profiles（逐 profile 模型/工具集覆盖 + profile home）；`--fix` 创建缺失的 home/子目录与默认 config.toml（hermes `--fix` 快速路径），`--online` 以阻塞 reqwest 探测提供商端点（bearer 密钥访问 `/v1/models`；ollama 类本地走 `/api/tags`），`--json` 输出序列化报告；问题汇总编号列出 + `--fix` 提示，与 hermes 一致恒以退出码 0 结束 |
| 会话洞察（`agent/insights.py`） | ✅ 核心 | `insights.rs` + `ulnclaw insights [--days N] [--source S] [--json]` CLI + REPL `/insights [days]` + 网关聊天 `/insights [N] [--days N] [--source S]` 斜杠命令：InsightsEngine 以第二个 WAL 读取连接分析 state.db —— 总览（会话/消息/工具调用数、输入/输出/总令牌、平均会话时长、活跃天数）、基于 models.dev 定价的美元成本估算（`get_model_info`，provider 取自当前配置作为提示，无定价显示 "cost unknown"）、按令牌排序的模型分解、来源分解（hermes 平台分解）、`role='tool'` 行的工具调用分解（前 30）、活动模式（按小时 + 周一起始的星期桶、峰值检测）、按令牌排行的前 5 会话（含标题/日期）；排除已归档会话，`--source` 过滤对齐 hermes，终端渲染带 █ 条形图（hermes `_bar_chart`）、`format_duration_compact` + K/M 令牌格式化，serde JSON 报告、技能使用分解（扫描 assistant `tool_calls` JSON 中的 `skill_view`/`skill_manage` 调用 —— 每技能加载/编辑计数 + 最后使用日期、汇总总数、排行 `top_skills`，hermes `_get_skill_usage`/`_compute_skill_breakdown` 语义）、`get_usage_breakdown` 工具+技能载荷（hermes 仪表盘路由形状）与紧凑 markdown `format_gateway` 渲染器（支撑网关 `/insights` 斜杠回复） |
| 宠物（`agent/pet/` + `hermes_cli/pets.py`） | ✅ 核心 | `src/pets.rs` + `ulnclaw pets list|install|select|show|off|scale|remove|doctor|hatch`：petdex 吉祥物引擎 —— 公共清单抓取（petdex.dev、300 秒进程内缓存 + 后台预热、资产下载锁定 petdex 主机）、按 profile 的 `<home>/pets/<slug>/` 存储（pet.json + 精灵图：安装/加载/列表/解析/重命名/删除/zip 导出/空闲帧缩略图，防穿越 slug）、图集行分类推断（8 行旧版 vs 9 行 Codex 图集）+ 状态别名（waving/jumping/running）、`derive_pet_state` 活动→动画映射（error→failed、celebrate→jump、completed→wave、awaiting-input→waiting、tool-running→run、reasoning→review）、四种终端渲染模式 —— kitty 图形协议（分块 APC 传输 + Unicode 占位符虚拟放置载荷与行/列变音符）、iTerm2 内联图像、手写 DEC sixel（中值切割 ≤255 色调色板量化）、带可读性下限的真彩 Unicode 半块回退 —— 由 `[display.pet]` 配置驱动（enabled/slug/scale 0.1–3.0/render_mode/unicode_cols），select/off/scale 持久化写入；LLM 宠物孵化流水线（`agent/pet/generate/` → `src/pets_atlas.rs` + `src/pets_generate.rs` + `ulnclaw pets hatch`）：基础草稿 → 锚定行条带生成 → 帧提取 → 图集合成/校验 → 商店注册，提示词与 hermes 逐字一致，色键背景移除（边缘泛洪填充 + 饱和键快速路径 + 空洞修补）、互相关单元配准/归一化、running-left 镜像、idle 兜底、每行 3 次尝试的 4 路并发列生成，`[pets]` 配置 OpenAI 兼容图像端点（image_base_url/image_api_key/image_model；密钥回退 OPENAI_API_KEY/ULNCLAW_API_KEY，模型回退 gpt-image-2），`--style` 风格提示（pixel/plush/clay/sticker/flat-vector/3d-toy/painterly/auto）、`--drafts N` 仅出草稿模式与 `--base <path>` 从图片孵化；REPL `/pet`（toggle/list/scale/off/<slug> 领养）+ `/hatch <description>` 斜杠命令（hermes cli_commands_mixin 语义，含进度打印）；P126 移植了桌面生成悬浮层：Tauri 外壳的孵化对话框（提示词 + 风格 + 草稿数 → 基础草稿网格挑选 → 实时行进度 → 精灵图预览 + 自动领养）基于新的网关孵化任务 API（`POST /api/pets/hatch`、`GET /api/pets/hatch/:id`、`POST /api/pets/hatch/:id/pick|cancel`、`GET /api/pets/hatch/:id/draft/:index`）。已知差异：单一 OpenAI 兼容端点取代 hermes 的 Nous/OpenRouter/Krea 供应商注册表；精灵图以 PNG 编码（`image` crate 无 WebP 编码器），解码两种格式均可 |
| 建议自动化（`cron/suggestions.py` + `suggestions_cmd.py`） | ✅ 核心 | `cron/suggestions.rs`：`<home>/cron/suggestions.json` JSON 存储（tmp+rename 属主权限写入），完整 hermes 语义 —— pending/accepted/dismissed 状态、dedup-key 锁存（已决策的键不再重复建议）、MAX_PENDING=5 积压上限、来源校验（catalog/blueprint/usage/integration）、按 id/1 基待处理序号/精确标题解析；`accept` 将存储的 job_spec 经 `CronStore` 落地为真实 cron 任务并置 accepted；`clear_resolved` 仅清除 accepted 记录（dismissed 保留作去重记忆）；4 条精选起始目录（每日简报、重要邮件监控、每周回顾、工作日开始提醒 —— 提示词改写为自包含，日程经 `parse_schedule` 验证），`seed_catalog_suggestions` 幂等；REPL `/suggestions [accept N|dismiss N|catalog|clear]` 与 `ulnclaw suggestions` CLI 共用 `handle_suggestions_command` 分发（accept/add/schedule 与 dismiss/no/reject 别名对齐、用法文本） |
| 状态报告（`hermes_cli/status.py` + `hermes_cli/subcommands/status.py` + `timefmt.py`） | ✅ 核心 | `status.rs`：`show_status` 移植 —— 面板头 + Environment（版本 / home / config.toml / .env）、Model+Provider+Base URL、API Keys（config.toml `model.api_key` 行 + 20 项供应商环境变量表，备选变量依次回退，所有值经 `redact_key` 脱敏）、Terminal Backend、Browser（endpoint + 浏览器发现）、Gateway（监听 / 鉴权密钥 / multiplex；`--deep` 追加网关端口 TCP 探测）、Scheduled Jobs（启用/总数 + 下次运行）、Sessions（总数 + 最近会话）、Skills（已安装 + 待处理建议）、Updates（git 上游检查，6h 缓存）、页脚指向 doctor/init；`relative_time()` 为 timefmt.py 移植（just now / Nm / Nh / yesterday / Nd / 日期）；CLI `ulnclaw status [--all] [--deep]`（`--all` 与默认渲染同为脱敏输出） |
| 日志查看 + 文件日志（`hermes_cli/logs.py` + `hermes_cli/subcommands/logs.py` + `hermes_logging.py` 滚动手柄） | ✅ 核心 | `logs.rs`：查看器移植 —— `LOG_FILES` 注册表（agent/errors/gateway）、`_parse_since`（Ns/m/h/d 截止时刻）、时间戳/级别/记录器名正则（记录器正则扩展以支持 Rust `::` 目标）、`_matches_filters`（level>= / session 子串 / since / 组件前缀）、`_read_last_n_lines`（<=1MiB 整读、大文件自尾部倍增分块）、`_read_tail`（带过滤时 20 倍窗口）、`list_logs`（大小 + 时龄表）、`tail_log` 头部/过滤描述对齐、`_follow_log` 300ms 轮询；写入端移植 —— `RotatingFile`（max_bytes x backup_count 移位轮转：agent.log 5MBx3 INFO+、errors.log 2MBx2 WARNING+、gateway.log 5MBx3 目标过滤）+ `HermesLogFormat`（`YYYY-MM-DD HH:MM:SS,mmm LEVEL [session] target: message`）经按文件 layer 接入 tracing；`COMPONENT_PREFIXES` 适配 ulnclaw 模块路径；CLI `ulnclaw logs [agent|errors|gateway|list] [-n] [-f] [--level] [--session] [--since] [--component]` |
| 自更新器（`hermes_cli/subcommands/update.py` + `update_cmd.py` git 核心） | ✅ 核心 | `update.rs`：`--check` 为 `_cmd_update_check` 移植 —— 分支解析（`--branch` > 当前分支 > master，hermes `_resolve_update_branch`）、浅克隆感知（`--depth 1` fetch + 仅比对 SHA 存在性）、默认分支优先 upstream fetch 并回退 origin、fetch 错误分类（网络 / 认证 / 通用）、比较引用校验、rev-list 落后计数；应用路径为 `_cmd_update_impl` git 核心移植 —— 自动 stash（`--include-untracked`、清理未合并索引、`ulnclaw-update-autostash-<ts>` 命名）、按 origin URL 检测 fork 并自动添加 `upstream` 远端（`_is_fork` / `_add_upstream_remote`，本地路径 origin 跳过）、`git merge --ff-only`（历史分叉只报告不强推）、stash 恢复与冲突指引、old..new 提交日志，随后 `cargo build --release` 作为 Rust 的依赖刷新等价物；Python 专属机制（venv/pip/npm、Windows 锁、Tauri/桌面、docker/nix、systemd 重启）对编译型 Rust 二进制不适用；CLI `ulnclaw update [--check] [--branch N] [-y]` |
| 备份与恢复（`hermes_cli/backup.py` + `hermes_cli/subcommands/backup.py`） | ✅ 核心 | `backup.rs`：完整 zip 备份（hermes `run_backup` —— 适配后的 `_EXCLUDED_DIRS/_SUFFIXES/_NAMES` 排除集、输出 zip 自排除、进度/错误摘要、`ulnclaw-backup-<ts>.zip` 命名、目录型输出处理），SQLite 经 `sqlite backup()` WAL 安全快照（`safe_copy_db`，hermes `_safe_copy_db`）+ `verify_sqlite_integrity`/`is_zeroed_sqlite_file`/`copy_db_and_verify`；导入（hermes `run_import` —— `validate_backup_zip` 标记文件校验、`detect_prefix` 含 `.ulnclaw`/`ulnclaw`、防 zip-slip 的暂存覆盖、`_IMPORT_SKIP_NAMES` 运行时状态保护、`_SECRET_FILE_NAMES` 0600 权限收紧）；快速快照（hermes `create/list/restore_quick_snapshot` + `_prune_quick_snapshots` —— manifest.json、防穿越 id、.db 原子替换、keep=20 修剪、pre-update 用 max_file_size 跳过）；cron 安全网 `restore_cron_jobs_if_emptied`（统计 state.db `cron_jobs` 表而非 jobs.json）；pre-update 钩子接入 `ulnclaw update`，`ulnclaw import` 前置 pre-import 快照 + 导入后安全网；CLI `ulnclaw backup [-o] [-q] [-l]` / `backup list|restore <id>|prune [keep]` / `ulnclaw import <zip>` |
| 回退链 CLI（`hermes_cli/fallback_cmd.py` + `fallback_config.py`） | ✅ 核心 | `fallback.rs`：运行时链已存在（`[model] fallbacks` 规格 + `agent::with_fallback_specs` / `parse_fallback_spec`）；本次补齐管理 CLI —— `list`（主模型 + 编号链，hermes `cmd_fallback_list` 文案）、`add <provider:model>`（经同部署比较拒绝主模型自身、拒绝完全重复，供应商大小写不敏感）、`remove <N|provider:model>`、`clear`（TTY 确认，`-y` 跳过）；存储经行级 config.toml 编辑写回（`save_chain`：在 `[model]` 段内替换/插入 `fallbacks = [...]`，保留注释与顺序，文件缺失时创建）；hermes 交互式选择器以显式规格参数替代（ulnclaw 无 curses 选择器）；CLI `ulnclaw fallback [list|add|remove|clear] [-y]` |
| 活跃会话租约（`hermes_cli/active_sessions.py`） | ✅ 核心 | `active_sessions.rs`：跨进程租约注册表 `<home>/runtime/active_sessions.json`，由 `active_sessions.lock` 上的 flock 保护（hermes `_FileLock`）；条目携带 lease_id/session_id/surface/pid + `/proc/<pid>/stat` 启动时刻，PID 复用无法伪造存活（hermes psutil create_time 配对）；`prune_dead` 在每次变更时回收死进程租约；`try_acquire/release/transfer_active_session`、`release_orphaned_leases`、`active_session_registry_snapshot`、`summarize_holders`（"desktop x4, cli, oldest Nh ago"）+ `active_session_limit_message` 文案对齐；上限经 `[gateway] max_concurrent_sessions` 配置（0/未设禁用；hermes 顶层/gateway.* 解析），在 chat REPL 启动时强制执行，租约随 Drop 释放（网关请求路径无状态、不按槽位限制） |
| 配置管理 CLI（`hermes_cli/config.py` config_command） | ✅ 核心 | `config_cmd.rs`：`show`（面板头 + 路径 + 全量配置，机密键经 `status::redact_key` 脱敏）、`get <key> [--json]`（config.toml 点路径；全大写键按 hermes `_is_env_config_key` 经进程环境变量 + `.env` 解析）、`set <key> <value> [--force]`（标量类型推断 bool/int/float/数组/表/字符串、嵌套表创建、未知段提示与 hermes 对齐；env 风格键写入 `.env`）、`unset <key>`（config.toml 或 `.env` 行移除）、`path` / `env-path`、`edit`（$EDITOR）；存储经 TOML 往返重写（TOML 取代 hermes YAML，注释丢失为已记录的取舍） |
| Shell 补全（`hermes_cli/completion.py`） | ✅ 核心 | `ulnclaw completion <shell>` 基于 clap_complete：bash / zsh / fish（hermes 集合）另加 elvish / powershell；从实时 clap 命令树生成，自动跟随子命令变化（hermes 遍历 argparse 树同理）；SIGPIPE 恢复默认处理，管道接入 `head` 时静默退出 |
| 环境转储与版本（`hermes_cli/dump.py`、`build_info.py`） | ✅ 核心 | `ulnclaw dump [--show-keys]`：纯文本、可直接粘贴的装机摘要——版本 + git SHA/提交日期、系统、profile、home、模型/provider、含 `TERMINAL_ENV` 覆盖提示的实际终端 backend、`api_keys:` set/not set/脱敏值并带"仅 shell 存在、`.env` 缺失"告警（托管后端只读 `.env` 不读登录 shell）、`features:` toolsets / MCP 服务器 / 记忆 provider / gateway 监听+鉴权 / cron 激活-总数 / 技能 / 检查点，以及非默认 `config_overrides:`；`ulnclaw version [--no-update-check]`：版本行 + 安装目录/方式 + 复用 `update --check` 机制的实时升级状态；无 git 安装回退读取内置 `.ulnclaw_build_sha` 标记（对标 hermes `.hermes_build_sha`） |
| 记忆 CLI（`main.py cmd_memory`） | ✅ 核心 | `ulnclaw memory`：分库状态（`memory/MEMORY.md` 代理笔记与 `memory/USER.md` 用户画像的条目数 + 字节数，二者会注入每轮系统提示词）；`ulnclaw memory reset [all|memory|user] [--yes]`：hermes 风格清除清单（`◆ 文件 (说明) — N 字节`）、非 `--yes` 时交互式输入 `yes` 确认、逐文件 `✓ Deleted` 报告；REPL 内 `/memory` 查看当前内容 |
| 审批模式 CLI（`hermes_cli/approval_mode.py`） | ✅ 核心 | `ulnclaw approvals [manual|smart|off]`：查看当前生效的终端审批模式，或经规范配置写入器持久化新模式（config.toml `approvals.mode`），写入后重新读回校验生效，并按 hermes 风格报告用法错误/受管配置失败；模式语义（`manual` 人工确认、`smart` 先问辅助守护 LLM、`off` 硬底线之外自动放行）与终端守卫一致 |
| 提示词体积诊断（`hermes_cli/prompt_size.py`） | ✅ 核心 | `ulnclaw prompt-size [--json]`：测量每次调用的固定负载——系统提示词按四层拆分（基础身份 / 持久记忆 / 环境 / 易变日期+模型）并给出字符数与字节数、记忆文件体积、工具数 + JSON schema KB、按 schema 从大到小排序的 toolset 清单（回答"想省 token 该关哪个"）、按 SKILL.md 从大到小排序的已装技能（技能按需加载、不在基础提示词内）；与 `Agent::effective_system_prompt` 共用 `agent::DEFAULT_SYSTEM_PROMPT` 及相同构件，数字与实际注入的提示词一致 |
| 调试分享包（`hermes_cli/debug.py`） | ✅ 核心 | `ulnclaw debug report [--lines N] [--no-redact] [--output DIR]`：以本地文件方式收集 hermes 风格分享包（不上传 pastebin）——`report.txt`（强制脱敏的 `ulnclaw dump` + agent/errors/gateway 日志尾部）加每份存在的完整日志，均带 dump 头部与脱敏横幅；每个文件一次快照同时派生摘要/全文（防轮转竞态），机密经脱敏引擎 + 邮箱掩码处理，支持 `.1` 轮转回退，绝不改动磁盘日志 |
| 技能束（`agent/skill_bundles.py`、`hermes_cli/bundles.py`） | ✅ 核心 | `ulnclaw bundles list|show|create|delete|reload`：`<home>/skill-bundles/` 下的 YAML 技能束，把一组技能合并加载（`name/description/skills/instruction`，缺省以文件名兜底、slug 归一化与技能一致、重名 slug 先到先得、坏 YAML 跳过不影响发现）；REPL `/<bundle> [指令]` 一次把全部成员技能的 SKILL.md 注入同一轮，带 hermes 风格头部（已载/缺失清单、束指令、用户指令），束优先于同名未知命令，连字符/下划线互通；缺失技能跳过并提示（与 `-s` 预载同样宽容） |
| 导入其他 Agent 配置（`hermes_cli/agent_import.py`） | ✅ 核心 | `ulnclaw import-agent [claude-code|codex] [--source DIR] [--dry-run] [--overwrite]`：detect→parse→map→apply，逐项记录 imported/skipped/conflict/error；claude-code：`CLAUDE.md` → `memory/MEMORY.md` 条目（标题成为上下文前缀、跳过代码块/表格、去重），`.claude.json` + `settings.json` 的 `mcpServers` → config.toml `[[mcp.servers]]`（同名冲突保留原配置、机密风格 env 变量剥离并报告），`skills/` → `skills/claude-code-imports/`，权限规则以转换后的命令模式报告（ulnclaw 无 allowlist 配置面）；codex：`AGENTS.md` + `memories/*.md` → 记忆条目，`config.toml [mcp_servers.*]` → `[[mcp.servers]]`，`skills/` → `skills/codex-imports/`；记忆合并前先备份（`.bak.<ts>`）、迁移预算 2 万字符；凭据文件绝不读取，dry-run 不写任何文件 |
| 会话技能标题修复（`hermes_cli/sessions_cmd.py retitle-skills`） | ✅ 核心 | `ulnclaw sessions retitle-skills [--limit N] [--apply]`（默认干跑）：`list_skill_scaffolded_sessions`（首个用户回合匹配 `[IMPORTANT: The user has invoked the` 脚手架且已有标题的会话）、`describe_skill_invocation` 从捆绑与单技能格式还原用户键入的调用（引号名称、`User instruction:` / `alongside the skill invocation:` 提取、摘录接缝切分、空白折叠）、`generate_title_forced` 绕过自动标题开关、`_is_titlelike` 拒绝命令输出型候选、唯一标题冲突经 `get_next_title_in_lineage` 去重（`base #2`、`#3`……） |
| Secrets 保险库（`agent/secret_sources/`） | ✅ 核心 | `src/secrets.rs` + `ulnclaw secrets status|sync [--apply]`：外部秘密源在启动时、任何 provider 读取 env 之前应用（hermes env-loader 钩子）。三个来源，完整复刻 hermes 优先级语义 —— mapped 优先于 bulk、首个声明者胜出、`preserve_existing` 胜过一切、`override_existing` 可覆盖已有 `.env`/shell 值但绝不覆盖其他来源、引导令牌变量写入保护。`command`：经 `/bin/sh -c` 的任意 KEY=VALUE 助手（keepassxc-cli / secret-tool / tmpfs cat），硬超时降级为“无值”，stderr 丢弃，1 MiB 输出上限，支持引号/注释解析。`bitwarden`：Bitwarden Secrets Manager，经 `bws secret list <project> --output json`（托管 `<home>/bin/bws` 优先于 PATH、`BWS_SERVER_URL` 透传、固定 v2.0.0 自动安装 —— 来自 bitwarden/sdk-sm releases 的 sha256 校验 zip、zip-slip 防护解包、0755 分阶段安装）。`onepassword`：映射式 `op://vault/item/field` 绑定，经 `op read -- <ref>` 解析，子进程仅继承最小允许清单 env，空值拒绝写入，单引用失败降级为警告。拉取错误只产生单行警告、绝不致命。TTL 拉取缓存（`agent/secret_sources/_cache.py` 移植，`src/secrets_cache.rs`）：`<home>/cache/` 下原子 0600 写入（目录 0700），TTL 为 0 时两层缓存对称关闭，仅完整无错拉取入缓存；Bitwarden 缓存落盘即 AES-256-GCM **加密**（HKDF-SHA256 密钥由引导令牌派生、缓存键绑定为 AAD、迁移成功后删除旧明文缓存）。交互式安装向导：`secrets bitwarden setup|install|status|token|disable`（hermes 五步流程 —— 安装二进制 → 令牌 → 区域 → `bws project list` 项目选择 → 测试拉取 → 保存配置；非 TTY 快速路径要求 `--access-token`/`--server-url`/`--project-id`）与 `secrets onepassword setup|status|set|remove|disable`。`secrets bitwarden token` 无需重跑向导即可轮换访问令牌（hermes `cmd_token`：掩码提示或 `--access-token`、`0.` 形状警告、以新凭据 `bws project list` 先验证后落盘（除非 `--no-verify`）、已配置项目可见性警告、写入 .env 并清除两层缓存）。未移植：Windows bws 资产路径未测试 |
| Computer Use（`tools/computer_use/`） | ✅ 核心 | `src/computer_use.rs` + `ulnclaw computer-use status|doctor|install`：经 cua-driver 守护进程的后台桌面控制（MCP over stdio，hermes `cua_backend.py`）。完整复刻 hermes 工具 schema（capture som/vision/ax、按 SOM 元素索引或坐标的 click 族、drag、scroll、type、组合键、set_value、wait、list_apps/list_windows/focus_app、cua_browser_* 类型化浏览器透传）。复刻 hermes 审批语义：capture 与列表类免费，其余动作一律走审批回调，无人值守时失败关闭。惰性共享 MCP 会话（`start_session`/`end_session`、`set_config` max_image_dimension、光标覆盖层策略含 `--no-overlay` 自动探测 + 默认 `CUA_DRIVER_RS_TELEMETRY_ENABLED=0`）。`doctor` 驱动 cua-driver 的 `health_report`。未移植：macOS TCC `permissions` 授权流程、嵌入式守护进程/socket 模式、截图驱逐 + 视觉后处理（驱动载荷直接透传） |
| 插件系统（`hermes_cli/plugins.py`、`agent/shell_hooks.py`） | ✅ 核心 | `src/plugins.rs` + `ulnclaw plugins list|enable|disable|accept-hooks`：以 shell-hook 线协议实现的 hermes 插件架构 Rust 原生化移植（静态二进制无法导入 Python 插件）。目录插件位于 `<home>/plugins/<name>/plugin.toml`（manifest：hooks + `[[tools]]`）；工具以 `plugin__<name>__<tool>` 注册，作为子进程运行、stdin 收 `{"tool", "arguments"}` JSON。配置式 shell 钩子 `[hooks] <event> = ["cmd"]`，复刻 hermes 首次使用同意机制（`shell-hooks-allowlist.json`、`auto_accept` / `ULNCLAW_ACCEPT_HOOKS`）。完整复刻 hermes `VALID_HOOKS` 目录（23 个事件）；核心触发 hermes 运行期实际发出的全部 13 个：`pre_tool_call`（block 决定在审批前否决）、`post_tool_call`、`transform_llm_output`、`on_session_start`/`on_session_end`/`on_session_reset`（`/new`）/`on_session_finalize`（REPL 退出）、`pre_llm_call`（context 响应追加进当轮用户消息，hermes turn-context 语义）、`post_llm_call`、`pre_api_request`/`post_api_request`/`api_request_error`（包裹每次 provider 调用）、`pre_gateway_dispatch`（在白名单门控之前 skip/rewrite 平台消息）；其余 10 个在 hermes v2026.8.3 中也仅存于目录。`ulnclaw hooks list|test|revoke|doctor`（hermes `hooks` CLI）检查同意状态、以默认载荷触发、逐个探测已同意的钩子。hermes 的 Python 插件导入、entry-point 包与 provider 注册未移植 |
| 消息平台网关（`gateway/platforms/`） | ✅ 核心 | `src/messaging.rs` —— hermes 平台网关架构运行于 `ulnclaw gateway` 内：适配器将入站聊天消息归一化为 `MessageEvent`，每个聊天一个会话（`platform-<name>-<chat>`，经 `create_named_session`）承载对话连续性，回复经平台送回并按 hermes 风格分块。十一个自包含适配器——七个长驻循环（Telegram（Bot API 长轮询 getUpdates/sendMessage）、Discord（Gateway v10 websocket IDENTIFY/心跳/MESSAGE_CREATE + REST 发送）、Slack（Socket Mode events_api 信封 + chat.postMessage）、Signal（signal-cli HTTP 守护进程：SSE 入站 + keepalive/闲置健康重连、JSON-RPC 2.0 出站 + 限流重试、Note-to-Self 提升与出站回声抑制、`group_allowed_users` 群组门控（`*` 通配）+ require-mention 过滤、附件经 `getAttachment` base64 + mime 嗅探 + ADTS→m4a ffmpeg 重封装、`MEDIA:` 回复以 `base64Attachments` 发送；`[messaging.signal]` 或 SIGNAL_HTTP_URL/SIGNAL_ACCOUNT）、微信（经腾讯 iLink Bot API 的微信个人号：长轮询 getupdates + 持久化 sync-buf 断点续传、消息 id + 内容指纹双重去重、DM/群组准入策略（pairing/allowlist/open/disabled）映射到 ulnclaw 白名单∪配对门控、磁盘持久化的按对端 context_token 回显存储 + 会话过期去令牌回退发送、双向 AES-128-ECB 加密 CDN 媒体（图片/视频/文件/语音，SSRF 主机白名单）、2000 字符 markdown 感知分块 + 易复制行折行 + 文本防抖批处理、getconfig 输入指示票据、`ulnclaw weixin login` 二维码登录；`[messaging.weixin]` 或 WEIXIN_ACCOUNT_ID/WEIXIN_TOKEN）、QQ（官方 QQ Bot API v2：WebSocket 网关 Hello/Identify/Resume/心跳 + hermes 关闭码语义（4004 刷新令牌、4006/4007/4009 重置会话、4008 限流退避、4914/4915 停止重连），C2C/群 @/频道/频道私信事件 + 300 秒消息去重，markdown（msg_type 2）或去格式纯文本回复、被动回复 msg_id 挂载 + `msg_seq` 生成，出站媒体 8 MB 以下走 base64 内联、更大走三步分块上传（upload_prepare → 预签名 COS PUT + upload_part_finish → complete，含日配额 40093002 与分片重试 40093001 处理），语音优先取 `asr_refer_text`、否则原音频入 `[stt]` 管道，引用消息（message_type 103）上下文合并，INTERACTION_CREATE 确认（内联键盘与扫码配置未移植）；`[messaging.qq]` 或 QQ_APP_ID/QQ_CLIENT_SECRET）、元宝（腾讯元宝 App 机器人：WebSocket 网关会话经 HMAC-SHA256 `sign-token` HTTP 握手引导（北京时间 +08:00 时间戳、签名令牌缓存）、手写 protobuf 线格式编解码（`src/yuanbao_proto.rs`：ConnMsg 信封、AUTH_BIND/BIND_ACK、ping、push 回执、30 秒私聊/群组心跳）、入站推送解码 + 每发送者 1.5 秒防抖、DM/群组准入策略（pairing/allowlist/open/disabled）映射到白名单∪配对门控、markdown 感知 4000 字符分块回复经 WS 发送（send-c2c/send-group）、复刻 hermes 不重连关闭码（4012/4013/4014/4018/4019/4021）、仅文本 —— 媒体/表情包通道未移植；`[messaging.yuanbao]` 或 YUANBAO_APP_ID/YUANBAO_APP_SECRET/YUANBAO_BOT_ID）外加四个挂载于网关的 webhook 平台（WhatsApp Cloud、Microsoft Graph 变更通知、通用 webhook 平台与 BlueBubbles，详见下文）。复刻 hermes 配对语义：每个平台均受白名单门控，空白名单失败关闭并记录待添加的 id。交互式配对码（hermes `gateway/pairing.py` 移植，`src/pairing.rs`）：未授权发送者收到 8 位 CSPRNG 配对码（存储为加盐 SHA-256、1 小时过期、每平台至多 3 个待审、每用户每 10 分钟一次请求、连续 5 次审批失败锁定平台 1 小时）；`ulnclaw pairing list|approve|revoke|clear-pending` 管理授权，获批用户与白名单在认证门控处取并集（`[messaging] pairing = true` 默认开启）。媒体附件（hermes media-cache 管道移植，`src/media_cache.rs`）：入站 Telegram photo/document/video/audio/voice（getFile 下载、照片取最大尺寸）、Discord `attachments`、Slack `files`（bot bearer 下载）按内容寻址缓存于 `<home>/media-cache/`（SHA-256 命名、hermes mime→ext 表、25 MB 上限），以路径引用 + vision_analyze/video_analyze/read_file 提示交付 agent（hermes 文本回退语义）；出站回复中的 `MEDIA:<路径>` 标签在 Telegram（sendPhoto/sendDocument）、Discord（multipart）与 Slack（现代 `files.getUploadURLExternal` → PUT → `files.completeUploadExternal` 流程）转为原生上传，纯媒体入站消息无需文本即可流转。WhatsApp Cloud（hermes `whatsapp_cloud.py` 移植，`src/webhook_platforms.rs`）：网关挂载 `/webhooks/whatsapp`，Meta 验证握手（hub.challenge 回显）、原始请求体 `X-Hub-Signature-256` HMAC 校验，文本 + image/document/audio/video/sticker 入站走同一白名单∪配对 + 插件门控（入站媒体经 Graph `/media` 对象下载、按 Meta 分型大小上限入内容寻址缓存、caption 作为消息文本），回复经 Graph API 分块发送，另支持两步式原生媒体发送（`/media` multipart 上传 → media-id 消息）。Microsoft Graph 变更通知接入（hermes `msgraph_webhook.py` 移植）：`/webhooks/msgraph` validationToken 回显 + 必填 clientState 校验，通知以资源级事件呈现（Teams/Outlook 拉取端未移植）。通用 webhook 平台（hermes `webhook.py` 移植）：`[messaging.webhook]` 路由挂载于 `/webhooks/hook/<name>`，多方案签名校验（Svix `svix-*` 头 + base64 `whsec_` 密钥、GitHub `X-Hub-Signature-256`、GitLab `X-Gitlab-Token`、时间戳绑定的通用 V2 且禁止降级 V1、旧版 V1、测试用 `INSECURE_NO_AUTH`），300 秒重放窗口，每路由固定窗口限流（默认 30 次/分钟），投递 id 幂等（`X-Webhook-Delivery-Id` 或 `svix-id`，1 小时 TTL），头部事件过滤（`X-Webhook-Event`/`X-GitHub-Event`/`X-Gitlab-Event`），`{event}`/`{body}` 提示词模板，投递目标（`log`/`telegram`/`discord`/`slack`/`whatsapp_cloud`）与 `deliver_only` 零 LLM 推送。BlueBubbles iMessage 桥（hermes `bluebubbles.py` 移植）：`[messaging.bluebubbles]` 在网关挂载 `/webhooks/bluebubbles`，密码认证（查询参数 —— BlueBubbles webhook 无法发送自定义头 —— 或 `x-password`/`x-guid`/`x-bluebubbles-guid` 头），JSON 载荷 + 表单编码回退，复刻 hermes 事件门控（仅 `new-message`/`message`/`updated-message`；自己发出的消息与 tapback 反应 2000–2005/3000–3005 静默确认），chat-GUID 解析经 LRU-500 缓存 + 严格 `chatIdentifier` 匹配（不做参与者回退 —— hermes #24157）并支持 v1.9+ `chats[0]` 提取，附件下载入内容寻址媒体缓存，回复按段落分块（4000 字符上限、地址目标经 `chat/new` 建聊），multipart 附件发送，启动时 ping + server-info + 幂等 webhook 注册。入站语音消息进入音频 STT 管道（见语音转写行）：转写文本以 🎙️ 消息回显并注入回合。交互式 clarify 提问在 WhatsApp 上渲染为原生按钮/列表（见交互式 clarify 行），Telegram/Discord/Slack 使用编号文本。内联键盘未移植；元宝目前仅支持文本（媒体/表情包通道未移植） |
| 交互式 clarify（`tools/clarify_gateway.py` + WhatsApp interactive） | ✅ 核心 | `src/clarify_gateway.rs` + 消息层集成 —— `clarify` 工具在消息会话中可用：提问登记于有上限的网关注册表（hermes state cap），按平台渲染（WhatsApp ≤3 选项用 `interactive.type=button`、4+ 用 `type=list` 并附 ✏️ Other 行，数字标签 + 正文完整选项文本，20/24/72 字符上限，`cl:<id>:<idx|other>` 按钮 id —— hermes `send_clarify` 布局），并阻塞回合直至应答。点按路由复刻 `_dispatch_interactive_reply` 语义：索引→选项文本解析、Other 切换文本捕获（`mark_awaiting_text` + ✏️ 提示）、未授权点按认领不分发、过期 id 回退为以按钮标题作文本分发。会话内下一条纯文本消息将应答等待中的 clarify 而非开启新回合（hermes `_maybe_intercept_clarify_text`）。非 WhatsApp 平台收到编号文本提问；`appr:`/`sc:` 前缀（网关审批/slash 确认）与 hermes 无等待者路径相同，回退为文本 |
| 语音转写（`tools/transcription_tools.py` + gateway STT 管道） | ✅ 核心 | `src/stt.rs` —— hermes 音频 STT 管道：`[stt]` 配置（enabled/echo_transcripts/provider/language + 各 provider 子块，默认值对齐 hermes），内置 provider `local_command`（命令逃生舱，`ULNCLAW_LOCAL_STT_COMMAND`）、`groq`（whisper-large-v3-turbo）、`openai`（whisper-1）、`mistral`（Voxtral）、`xai`、`elevenlabs`（Scribe）、`deepinfra`（在线目录模型发现），均为 OpenAI 兼容 multipart 上传；自定义命令 provider 经 `[stt.providers.<name>]` + 旧式顶层块，保持内置名永远优先的不变式；网关语音消息（audio/* 附件）在回合前转写，复刻 hermes 语义 —— provider 失败时回退本地命令、空转写哨兵（#41603）、中性失败标记不进提示词、`🎙️ "<转写>"` 回显（stt.echo_transcripts）、STT 关闭时 WAV/ffprobe 时长注记；`transcribe_audio` agent 工具（可选 `stt` 工具集）支持 model/language 覆盖。已知差异：hermes 默认 `local` provider（faster-whisper，Python）无法嵌入静态二进制 —— 以 `stt.local.command` 或云 provider 替代 |
| OAuth 登录 + 技能同步（`hermes_cli/portal_cli.py`、`tools/skills_sync_client.py`） | ✅ 核心 | `src/oauth.rs` + `src/skills_sync.rs`：hermes 门户认证 + Skill Sync 的服务无关移植。`ulnclaw auth login` 对任意配置的 `[oauth]` provider（device_authorization_url/token_url/client_id/scopes）执行 RFC 8628 设备授权许可，处理 authorization_pending/slow_down，令牌存于 `oauth_tokens.json`（0600）、refresh-token 许可、`status`/`refresh`/`logout`/`open`。`ulnclaw sync status|pull|push|now|enable|disable|device` 原样保留 hermes 的 UX：可选技能同步 + 稳定设备 id + 设备标签，`[sync] base_url` 未配置时报告 INERT 门控，pull 绝不覆盖本地技能。传输通用：HTTP(S) REST（bearer = OAuth 令牌或 `[sync] api_key`）或共享目录（离线/NAS 同步）。Nous 门户专属的订阅特性与组织提案审批流程未移植 |
| 桌面 GUI（`apps/desktop` Electron） | ✅ 核心 | `desktop/` —— 以 Tauri 2 外壳取代 hermes 的 Electron 应用（用户指定）：Vite/TypeScript webview 聊天界面（会话侧栏悬停出现重命名 ✎ / 删除 🗑 操作，对接 `PATCH`/`DELETE /api/sessions/:id`；SSE 令牌流式 + 实时工具进度条，解析命名事件 `hermes.tool.progress` 渲染为 `⚙ <tool> — <status>`；设置、`/` 斜杠补全弹层（数据来自 `/v1/skills` + 网关命令集）、可展开工具调用卡片（`hermes.tool.started`/`hermes.tool.completed` SSE 事件，含参数 + 结果面板）、剪贴板图片粘贴经 `POST /api/uploads` 上传并以 hermes 文本回退媒体路径引用附加）直接以纯 HTTP/SSE 与 gateway 通信（`/api/sessions`、`/api/chat` 流式、`/api/config`）—— 无定制桥接协议；Rust 侧仅管理 `ulnclaw gateway` 子进程（二进制查找 PATH → `~/.local/bin`/`~/bin`/`~/.cargo/bin`，端口取自 `[gateway] port`，启动/SIGTERM 退出）。gateway 侧 `serve_multiplex` 增加面向本地应用的宽松 CORS 层（回显 Origin + OPTIONS 预检；依旧绑定 loopback 并校验 API key）。无 Tauri IPC 桥时自动退回浏览器模式。P119 新增看板挂件（基于 `/api/kanban/*` 的四列卡片墙：快速添加、完成/阻塞/解除阻塞、评论抽屉、看板切换、5 秒轮询）；P121 新增 petdex 宠物悬浮层（基于 `/api/pets/config` + `/api/pets/:slug/spritesheet` 的精灵图动画画布，`display.pet.*` 驱动，依据 `/v1/runs` 切换工作/空闲状态，点击挥手）及网关宠物 API；P126 新增孵化悬浮层（hermes pet-generate 对位：提示词/风格/草稿数表单、草稿网格挑选、实时行进度、精灵图预览），基于新的网关孵化任务 API（`POST /api/pets/hatch` → 草稿挑选 → 轮询 → 领养）；Electron 应用其余仪表盘挂件与托盘集成未移植 |
| 会话浏览（`hermes_cli/sessions_cmd.py browse` + curses 挑选器） | ✅ 核心 | `ulnclaw sessions browse [--source S] [--limit N]`：TTY 上启用原始模式 TUI（crossterm 移植 curses 挑选器 —— 备用屏幕、↑/↓/PgUp/PgDn/Home/End 滚动导航、键入即过滤 + 退格、绿色 `▶` 选中高亮、Enter 选择、无过滤时裸按 `q` 退出、Esc 先清空过滤再按一次才退出、单步 ↑/↓ 在列表首尾回绕、暗色列头（Title/Preview · Active · Src · ID）与底部页脚（光标位置 + 过滤前总数）、"终端过小"保护、LF（`Ctrl+J`）形式的 Enter 同样接受）；管道/CI 场景回退为编号 stdin 挑选器；按最近活动排序的行（标题 → 首条用户消息预览回退、相对时间、来源、截断 id）、对标题/预览/id/来源的子串过滤、未指定 `--source` 时排除 `tool` 源会话（hermes 语义）；选中后以 `--resume <id>` 重新启动当前二进制（hermes `relaunch`）；存储查询 `list_sessions_for_browse` 单条 SQL 返回挑选器行 |
| 会话恢复与单会话连续性（`cli.py --resume/--continue`） | ✅ 核心 | 全局 `-r/--resume <id或前缀>` 与 `-c/--continue` 标志，适用于 `chat` 与 `run`：整个 REPL 会话存于同一条会话记录（此前每轮都会新建记录）；恢复时用 `load_messages` 回填 REPL 历史（丢弃 system 行），打印 `Resuming session: <id> (标题)`，每轮经 `run_with_session` 写入同一 id；`/new` 轮换到新会话键并重置按会话的目标管理器；`latest_session_id` 按最近活动挑选 `--continue` 目标（跳过已归档）；所有带 id 的 `sessions` 动作（`show`/`export`/`recap`/`delete`/`rename`）均经 `resolve_session_id` 接受唯一前缀；hermes 的 `-c <会话名>` 标题查找不移植（`-c` 不带值） |
| 会话库修复（`hermes_state.py repair_state_db_schema`） | ✅ 核心 | `ulnclaw sessions repair [--check-only] [--no-backup]`：健康探测（`db_opens_cleanly` —— `PRAGMA journal_mode` 首语句触发、`integrity_check`、sessions 读取、FTS MATCH 读探测、回滚式 FTS 写探测）后按破坏程度逐级升级 —— FTS5 `'rebuild'` 原地重建、`REINDEX` 修复过期 B 树索引、经 `writable_schema` 去重 `sqlite_master`（保留 FTS 索引）、删除 FTS 结构 + `VACUUM` 并在下次打开时重建（`initialize_schema` 回填滞后的外联内容索引）；先做带时间戳的原始备份 + WAL/SHM 附属文件；失败时指向离线 `sessions recover`；在打开存储之前执行，因为库结构损坏正是无法打开的情形 |
| 会话删除/重命名/优化（`hermes_cli/sessions_cmd.py`） | ✅ 核心 | `ulnclaw sessions delete <id> [--yes]`（id 或唯一前缀，`resolve_session_id` —— LIKE 转义前缀匹配、精确 id 优先、歧义即未找到；除 `--yes` 外 y/N 确认；先删消息 + FTS 行再删会话）、`sessions rename <id> <title...>`（hermes `sanitize_title`：剥离 ASCII/Unicode 控制字符、折叠空白、空标题清除、100 字符上限、跨会话标题唯一；回报实际存储标题）、`sessions optimize`（FTS5 `'optimize'` 段合并 + 尽力 WAL checkpoint + `VACUUM`；报告合并索引数与前后大小，用 `logical_size_bytes` 页统计避免 WAL 滞后误报） |
| 供应链安全审计（`hermes_cli/security_audit.py`） | ✅ 核心 | `ulnclaw security audit [--json]`：按需对固定版本的 MCP 服务器包做 OSV.dev 审计（`npx pkg@ver` / `uvx pkg==ver`，含 npm 作用域包）；未固定版本/本地条目静默跳过不猜测；`querybatch` + 逐漏洞详情抓取（severity 取 `database_specific`/`ecosystem_specific`、修复版本去重、摘要截断 100 字符）；结果按严重度排序、按来源分组，人类可读 + JSON 输出；hermes 的 venv/插件扫描面对静态 Rust 二进制不适用 |

## 功能对标

| hermes 功能 | ulnclaw | 说明 |
|---|---|---|
| 工具调用代理循环 | ✅ | 迭代预算、用量统计、step 回调 |
| SQLite 状态库（`hermes_state.py`） | ✅ | sessions/messages/system_prompts/state_meta/async_delegations 表结构，FTS5（不可用时 LIKE 回退），会话血缘 |
| 会话数据库恢复（`session_recovery.py`） | ✅ 核心 | `ulnclaw sessions recover <db> [--out FILE]`：离线、非破坏性——源库连同 WAL/SHM/journal 旁车文件复制到一次性目录，规范表按列交集拷入全新当前表结构库，受损表按 rowid 逐行抢救，孤儿消息重建会话行，重建 FTS，完整性校验 + JSON 报告；绝不就地修复或覆盖在用数据库 |
| 环境探针（`tools/env_probe.py`） | ✅ | 终端后端为本地时，向系统提示注入一行确定性的 Python 工具链说明：python3/python 版本、pip 模块可用性、`pip`↔`python3` 版本错配、PEP 668 外部管理标记（有 uv 时不告警）；健康环境保持静默；进程级缓存由单一后台线程构建，调用方最多等 10 秒后放行；远端后端（docker/ssh）跳过探测；`[agent] environment_probe` 开关（默认开启） |
| 上下文压缩（`conversation_compression.py`） | ✅ | 预算触发，中段对话经二次模型调用摘要，保留系统提示词 + 首条用户消息 + 最近尾部；摘要调用遵循 `[auxiliary.compression]` 路由 |
| 流式思考块清洗（`agent/think_scrubber.py`） | ✅ | `think_scrubber.rs`：对流式增量中的 `<think>`/`<thinking>`/`<reasoning>`/`<thought>`/`<REASONING_SCRATCHPAD>` 块做有状态抑制 —— `call_with()` 中每个内容增量都经过状态机喂送（开标签可跨增量分片存活，未闭合开标签受块边界门控），流结束时冲刷暂留的部分标签尾部，非流式路径走完整字符串 `strip_think_blocks`；闭合对总是被抑制，开标签仅在块边界生效，因此仅提及标签名的正文不会被误剥离 |
| 会话标题生成器（`agent/title_generator.py`） | ✅ | `title_generator.rs`：首轮交流后即发即忘的自动标题（后台任务，不增加回复延迟）—— 前 2 轮用户消息守卫、已有标题守卫、`[auxiliary.title_generation]` 路由（`language` 语言固定、`enabled` 开关，`is_truthy_value` 语义，默认 true）；500 字符摘要、答案先经推理块清洗、引号/"Title:" 前缀/首行/80 字符清理；`set_auto_title_if_empty` 原子持久化 —— 生成进行中手动设置的标题优先保留；可选标题回调与 portal/记账标签未移植（无对应实时 UI 面） |
| 持久化目标 —— Ralph 循环（`hermes_cli/goals.py`） | ✅ 核心 | `goals.rs`：跨轮次存续的既定目标 —— 每个助手轮次结束后由 `goal_judge` 辅助模型裁决 done/continue/wait；未完成时把续跑提示作为普通用户消息回灌，直至目标达成、被暂停/清除或轮次预算（默认 20）耗尽。完成契约（outcome/verification/constraints/boundaries/stop_when）可通过内联 `field: value` 目标行或 `/goal draft`（辅助模型起草）设定；子目标（`/subgoal`）并入裁判与续跑提示；WAIT 裁决把循环停泊在后台进程 pid/会话或截止时间上而不耗轮次（`/goal wait <pid>`，条件满足自动解除）；裁判 fail-open，解析连续 3 次或传输连续 5 次失败自动暂停；目标状态持久化于 `state_meta`（键 `goal:<session_id>`，重启后仍在，`migrate_goal_to_session` 支持会话轮换）；REPL `/goal` + `/subgoal` 斜杠命令；kanban 目标循环留在桌面侧 |
| 时区感知时钟（`hermes_time.py`） | ✅ | `hermes_time.rs`：IANA 时区解析顺序 `ULNCLAW_TIMEZONE` → `HERMES_TIMEZONE` → 配置 `timezone` → 服务器本地时间（非法时区名告警后回退，进程级缓存 + `reset_cache()`）；系统提示词注入仅含日期的 "Conversation started" 行 + Model/Provider（全天字节稳定以保护前缀缓存，hermes PR #20451）；压缩摘要携带 `Current date` 时间锚点 |
| 审批系统（`approval.py`） | ✅ | 命令归一化（反斜杠续行、`${IFS}`、注释剥离）、硬性底线（直接阻止）、可恢复但昂贵的操作（需确认）；REPL y/N 提示；网关运行审批（`POST /v1/runs/:id/approval`，once/session/always/deny，SSE `approval.request`）、fail-closed `[approvals] timeout`（默认 300s）、`always` 授权跨重启持久化；`[approvals] mode = manual|smart|off` —— smart 模式先询问辅助守护 LLM（防提示注入的提示词设计，运维 `smart_policy` 仅走可信通道），不确定时升级人工，`off` 在硬性底线以下自动放行；`cron_mode = deny|approve` 管控无人值守 cron 运行（deny = fail-closed 默认） |
| 威胁模式扫描（`threat_patterns.py`） | ✅ 核心 | 对重新进入上下文的工具结果做提示注入扫描（建议性） |
| 工具集（`toolsets.py`） | ✅ | 全部 33 个工具集定义，含组合（`includes`），默认 `coding` |
| 工具注册表（`registry.py`） | ✅ | check_fn 门控、工具集分组、结果大小截断 |
| Provider 抽象（`runtime_provider.py`） | ✅ | OpenAI 兼容（OpenAI/OpenRouter/DashScope/Ollama/llama.cpp）、原生 Anthropic Messages 传输（`anthropic_messages`：system 参数、tool_use/tool_result 块、SSE 流式、max_tokens 上限、OAuth bearer）、本地 provider 免密钥 |
| Provider 回退链（`fallback_providers`、`try_activate_fallback`） | ✅ 核心 | `[model] fallbacks = ["provider:model", ...]`：模型调用失败时按序推进（每条目惰性构建客户端、密钥回退主运行时），激活的回退在本轮内保持生效，下一轮恢复主 provider（hermes `restore_primary_runtime`）；委派/cron 子代理继承配置 |
| 辅助模型路由（`auxiliary_client.py`） | ✅ 核心 | `[auxiliary.<task>]` 按任务覆盖 provider/模型/base_url/api_key/key_env（`compression`、`vision`、`title_generation`）；`"auto"`/留空继承主运行时；无覆盖时复用主客户端 |
| models.dev 目录（`agent/models_dev.py`） | ✅ 核心 | `models_dev.rs`：拉取 `https://models.dev/api.json`，三级缓存——内存（1 小时 TTL，过期数据立即返回并由后台线程刷新）→ 磁盘（`$ULNCLAW_HOME/models_dev_cache.json`，任意陈旧度可用）→ 网络单飞获取（失败后进程级退避 5 分钟）；provider ID 映射 + 同名回退、上下文/能力查询（大小写不敏感、`:cloud`/`-cloud` 后缀回退）、agentic 目录过滤（噪声模式 + Google 隐藏清单）、`get_provider_info`/`get_model_info`；`ULNCLAW_MODELS_DEV_URL` 镜像覆盖（http(s)/file）、`ULNCLAW_MODELS_DEV_CACHE` 路径覆盖；网关 `/api/model/options` 目录增强 + `?refresh=true`；CLI `ulnclaw models providers\|list\|info\|refresh` |
| 配置（`config.yaml`） | ✅ | `config.toml` + `.env` 文件、profiles、环境变量优先级 |
| 技能系统 | ✅ | 发现、frontmatter、关联文件、`/skill-name` 调用脚手架（hermes `build_skill_invocation_message` 移植：激活注记 + 技能正文 + 技能目录/辅助文件提示 + 用户指令标记、`skill_usage` 计数；供 `sessions retitle-skills` 识别） |
| 记忆系统 | ✅ | MEMORY.md/USER.md，注入提示词 |
| Cron 调度器 | ✅ | 任务存储 + 计划解析 + 轮询循环（`cron::run_scheduler`） |
| MCP 客户端（`mcp_tool.py`） | ✅ 核心 | stdio JSON-RPC：initialize/tools/list/tools/call；`[[mcp.servers]]` 配置；工具注册为 `mcp__<server>__<tool>`；npx/uvx/pipx 启动前的 OSV 恶意软件检查（`osv_check.py` 移植：MAL-* 通告阻止启动、fail-open、1 小时结论缓存、`OSV_ENDPOINT`/`OSV_CHECK_CACHE_TTL` 覆盖） |
| CLI（`hermes_cli/`） | ✅ 核心 | 带斜杠命令的聊天 REPL（含 `/rollback [N|hash] [file]`、`/rollback diff <N>`、`/diff` 检查点命令、`/recap`、`/goal` + `/subgoal` 既定目标循环、`/kanban` 看板内联操作、`/pet` + `/hatch` petdex 界面）、一次性 `run`、sessions/tools/skills/cron/checkpoints 子命令（含 `sessions export --format md\|html` —— SHA256 校验的 Markdown 或独立 HTML + manifest ——、`sessions recap`、`sessions recover`、`sessions prune`/`archive`/`stats`/`delete`/`rename`/`optimize`/`repair`/`browse`/`retitle-skills`、`kanban init`/`boards list|create|rm|switch|show`/`create`/`list`/`show`/`ready`/`assign`/`claim`/`heartbeat`/`done`/`block`/`unblock`/`archive`/`comment`/`link`/`unlink`/`dispatch [--max-spawn N] [--dry-run]`/`gc`/`swarm <goal> --worker ASSIGNEE:TITLE[:skill,skill] [--worker ...] --verifier ASSIGNEE --synthesizer ASSIGNEE [--idempotency-key K] [--json]`/`specify [id | --all]`/`decompose [id | --all]`/`diagnostics [id] [--min-severity S] [--json]`/`schedule`/`promote [--force]`/`reclaim`/`reassign [--reclaim]`/`edit`/`set-model`/`attach`|`attachments`|`attach-rm`/`tail [--follow]`/`stats [--json]`/`watch [--assignee P] [--kinds K] [--interval S]`（kanban 任务引擎：boards、带 TTL 的认领锁 + 过期接管、带图标的 hermes 状态生命周期、评论与事件轨迹、`kanban_task_*` 插件钩子）、`secrets status/sync/bitwarden setup|install|status|disable/onepassword setup|status|set|remove|disable`、`computer-use status/doctor/install`、`plugins list/enable/disable/accept-hooks`、`hooks list/test/revoke/doctor`、`pairing list/approve/revoke/clear-pending`、`weixin login`（微信 iLink 扫码登录）、`auth login/status/refresh/logout`、`sync status/pull/push/now/enable/disable/device`、`uninstall --full/--dry-run/--yes`（代码检出 + shell PATH 条目 + 包装符号链接 + 可选清除主目录；hermes `uninstall.py` 移植 —— Windows 注册表/环境变量步骤未移植））、`moa run/list/delete`、`models providers/list/info/refresh`（models.dev 目录）、`skills blueprints/schedule/unschedule`、`diff`、`init` |
| Git 工作区 diff（`working_diff.py`） | ✅ | `ulnclaw diff [--staged|--all] [--dir PATH] [paths...]` + REPL `/gitdiff [staged|all]`：working/staged/all 三模式，未跟踪文件经 `git diff --no-index` 折入（上限 50 个），带超时；基于检查点的 REPL `/diff` 保持独立 |
| 委派（delegation） | ✅ | SubAgentRunner trait、深度限制、子会话 |
| 混合智能体 MoA（`moa_loop.py`、`moa_config.py`） | ✅ 核心 | `[moa.presets.<name>]` 参考模型并行扇出 + 聚合器综合（`ulnclaw moa run/list/delete`、REPL `/moa <prompt>`）；loud/silent 降级策略、全部失败提前返回、聚合失败回退拼接结果；持久 `provider: moa` 门面、trace 与隐私过滤未移植 |
| HTTP 网关（`gateway/platforms/api_server.py`） | ✅ 核心 | `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions`（`X-Ulnclaw-Session-Id` 会话续接、`stream: true` SSE 令牌流 + `hermes.tool.progress` 事件）、`/v1/responses`（经 `previous_response_id` 有状态续接、`stream: true` Responses-API SSE 事件）、`/v1/models`、`/api/model/options`（models.dev 目录增强、`?refresh=true`）、`/v1/capabilities`、`/v1/runs`（异步运行 + SSE 事件 + 停止 + 审批）、`/api/sessions` 增删查改 + 会话聊天 + chat/stream（斜杠直通：`/help`/`/skills`/`/tools`/`/recap`/`/title`/`/usage` 免 LLM 回合直接执行；`/skill-name` + `/<bundle>` 调用展开为 hermes 技能脚手架用户轮——复刻 hermes gateway/run.py 的技能命令共享）+ `PATCH`（title/end_reason）+ `fork` + 会话级模型锁（每轮生效）+ `recap`、`/api/jobs` 定时任务 HTTP API（增删查改 + pause/resume/run）、`/v1/skills`、`/v1/toolsets`、`/metrics`（Prometheus 计数器/量表——ulnclaw 运维扩展）、`/api/usage`（令牌核算：进程计数器 + 全时会话库总量 + 按会话明细——ulnclaw 运维扩展）、`/v1/delegations`（后台委派登记——ulnclaw 运维扩展）、`/v1/browser/status|connect|disconnect`（实时 CDP 端点控制，对齐 hermes `/browser connect`——ulnclaw 运维扩展）、`/api/uploads`（二进制上传入内容寻址媒体缓存——桌面剪贴板图片粘贴）、Bearer 令牌鉴权 |
| TUI/web/app | ✅ 核心 | TUI：聊天 REPL + 原始模式会话挑选器（`sessions browse`）；web：HTTP 网关提供 OpenAI 兼容 + 会话 API 并带本地应用 CORS，任意浏览器仪表盘可直接接入；桌面：Tauri 外壳（`desktop/`，见桌面 GUI 行） |
| 沙箱环境清洗 + passthrough（`environments/local.py` 黑名单、`env_passthrough.py`） | ✅ | terminal/execute_code 子进程继承的环境会剔除 provider/工具凭证黑名单与虚拟环境标记（`VIRTUAL_ENV`/`CONDA_PREFIX`）；技能 `required_environment_variables`（`skill_view` 时注册）与 `[terminal] env_passthrough` 放行其余变量——受保护的 provider 凭证与 `AUXILIARY_*_API_KEY`/`GATEWAY_RELAY_*` 动态密钥永远被拒绝（hermes GHSA-rhgp-j443-p4rf，失败即关闭） |
| 环境（`tools/environments/`） | ✅ 核心 | `terminal` 后端：local（默认）、docker（`ensure_docker_container` inspect→run）、ssh（BatchMode、identity 文件）；`[terminal] backend/container/image/ssh_host/...`；modal/daytona/vercel 暂缓 |
| 检查点管理器（`checkpoint_manager.py`） | ✅ | v2 共享 shadow git 存储（`<home>/checkpoints/store`）：按项目 ref/index，编辑前透明快照（每轮 `write_file`/`patch` 前一次），list/restore/diff/prune CLI，容量上限、超大文件过滤、孤儿/过期自动清理 |
| 浏览器监督器 | ✅ | `ULNCLAW_BROWSER_CDP=auto` 时自动启动受管 headless Chrome/Chromium |
| Camofox 后端（`tools/browser_camofox.py`） | ✅ 核心 | `browser/camofox.rs`：`CAMOFOX_URL` REST 反检测浏览器（Camoufox）后端——全部 12 个 browser 工具经 REST 路由（标签页会话、带元素引用的可访问性快照、点击/输入/滚动/后退/按键、从快照提取图片、截图供视觉分析）；CDP 覆盖优先；`CAMOFOX_API_KEY` bearer 鉴权、`CAMOFOX_USER_ID`/`CAMOFOX_SESSION_KEY` 身份覆盖 + 已有标签页收养、Docker 环回 URL 重写（`CAMOFOX_REWRITE_LOOPBACK_URLS` + 别名）、从 `/health` 发现 VNC URL、读取操作的 SSRF 私有页面防护、console/原始 CDP/对话框明确报不支持；`CAMOFOX_MANAGED_PERSISTENCE` 受管持久化（稳定的 UUIDv5 profile 级 userId，对应 hermes `browser.camofox.managed_persistence`）；网关与 REPL browser status 报告后端 |

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
├── media-cache/            内容寻址的消息平台媒体缓存
├── pairing/                DM 配对存储（{platform}-pending/approved.json）
├── shell-hooks-allowlist.json   钩子同意记录
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
- 网关实现了 api_server 平台的子集（多 profile 复用
  `/p/<profile>/...` 已移植 —— 见功能表）。任务 API 仅本地投递
  （`deliver="local"`）；hermes 的外部投递目标与 NAS/Chronos 触发
  webhook（`/api/cron/fire`）未移植。
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
- Secrets：`secrets sync` 干跑对比的是启动钩子已应用后的 env（hermes 行为
  相同）；Bitwarden `token` 轮换子命令未移植（改令牌仍可直接编辑 .env 后
  重跑 setup）；bws 自动安装的 Windows 资产路径未测试。
- Computer-use：驱动载荷（SOM 截图 b64、AX 树）直接透传，不含 hermes 的 PNG
  后处理/多模态驱逐层；macOS TCC `permissions` 授权流程与嵌入式守护进程
  socket 模式未移植；`install` 直接调用上游 trycua 安装脚本。
- 插件：ulnclaw 插件是讲 hermes shell-hook JSON 协议的子进程（目录插件 +
  `[hooks]` 配置），不是 Python 导入；核心触发 hermes v2026.8.3 运行期
  实际发出的全部钩子（23 个中的 13 个 —— 其余 10 个在 hermes 中也仅存于
  目录）；pre_verify 无 ulnclaw verify 循环可挂载；`ulnclaw kanban` 引擎
  现已在 claim/done/block 时触发 kanban_task_claimed/completed/blocked
  钩子，agent 侧 kanban_* 工具现已改用同一 KanbanStore 引擎
  （P119 统一了此前独立的表）；P122 移植了调度器 tick（`kanban dispatch` CLI +
  `POST /api/kanban/dispatch`：过期认领回收（存活 pid 自动续期）、父任务完成的
  todo→ready 晋升、就绪任务经分离的 `ulnclaw run` 生成 worker（ULNCLAW_KANBAN_TASK
  环境）、实时并发上限、连续 2 次 spawn 失败自动阻塞）；P123 新增网关内嵌定时调度
  （`[kanban] dispatch_in_gateway / dispatch_interval_secs / max_spawn`，默认开/60 秒/2）
  与 hermes kanban-stop 提醒（worker 未调用 kanban_complete/block 就结束时最多重新
  提示 2 次，`ULNCLAW_KANBAN_STOP_NUDGE=0` 可关闭）；P124 补齐了按任务
  git-worktree 隔离（`[kanban] worktrees`，默认开启：每个被调度的 worker 运行在
  `<repo>/.worktrees/<task-id>` 的 `kanban/<task-id>` 分支上，重启可复用；
  `ulnclaw kanban gc` 清理已完成/归档任务的树，分支保留）；P125 移植了 hermes
  kanban swarm（`hermes_cli/kanban_swarm.py`）：`ulnclaw kanban swarm <goal>
  --worker ASSIGNEE:TITLE [--worker ...] --verifier ASSIGNEE --synthesizer
  ASSIGNEE [--json]` 构建 workers→verifier→synthesizer 任务图 ——
  根黑板/审计任务（创建即 done）、N 个注入 swarm 协议简报的 ready worker、
  链接到所有 worker 的验证者、链接到验证者的综合者；拓扑以 `blackboard` 评论 +
  `swarm` 事件发布，调度器随父任务完成逐级晋升验证者/综合者
  （`recompute_ready`）；P127 补齐 swarm 面：worker 技能透传
  （`--worker ASSIGNEE:TITLE:skill,skill`，验证者固定
  `requesting-code-review`、综合者固定 `humanizer` —— 与 hermes 逐字一致）、
  任务级 `skills`/`max_runtime_seconds`/`idempotency_key` 列（增量迁移；
  `kanban create --skill X --max-runtime N --idempotency-key K`，网关创建
  API 同字段）、幂等 swarm 恢复（同键 ⇒ 从根黑板重建拓扑，不重复建图）、
  调度器 `reap_timed_out`（SIGTERM + 5 秒宽限 + SIGKILL，任务回 ready 并记
  `timed_out` 事件），派生 worker 的启动提示词内联强制加载的技能
  （hermes 以 `--skills` 参数对传递）；P128 移植了 triage 流水线
  （`hermes_cli/kanban_specify.py` + `kanban_decompose.py`）：
  `kanban create --triage` 把想法暂存进新的 `triage` 列，`kanban specify`
  经 `auxiliary.triage_specifier` 生成 Goal/Approach/Acceptance-criteria
  规格并晋升 triage→todo，`kanban decompose` 将其扇出为 2-6 个子任务
  依赖图并按 profile 名册路由（`[kanban] orchestrator_profile /
  default_assignee / auto_promote_children`；根任务作为所有子任务的父级
  存留作唤醒卡，Kahn 环检测，--all 批量时对单任务失败容错），
  `kanban diagnostics` 移植 `kanban_diagnostics.py` 规则引擎（幻觉卡号、
  正文幻影引用、重复 spawn 失败、worker 崩溃循环、阻塞超 24 小时、
  block/unblock 循环、滞留 ready、triage 无辅助模型），阈值与严重度
  排序与 hermes 一致；P129 补齐了其余 hermes kanban CLI 面：
  schedule/promote（父任务门控、--force 覆盖）/reclaim/reassign
  （--reclaim）/edit/set-model、附件 CLI（attach/attachments/attach-rm，
  稳定 id）、tail --follow 事件流、按看板状态统计、boards
  rename/set-workdir；P130 新增全板 `kanban watch` 实时事件流
  （assignee/kind 过滤，hermes watch 后端）、`kanban stats` 采用 hermes
  `board_stats` 语义（按 assignee 统计 + 最老 ready 等待时长 +
  `--json`）与 `kanban dispatch --json`；P131 移植网关通知底座：
  `kanban_notify_subs` 表（task × platform × chat × thread 主键，
  订阅时游标快照到当前最新事件、chat_type/profile/metadata 自愈）、
  `kanban notify-subscribe / notify-list / notify-unsubscribe` CLI、
  供网关通知器使用的 `unseen_events_for_sub` + `advance_notify_cursor`
  构件，以及 `kanban log [--tail N]` —— 打印任务在
  `<home>/kanban/worker-logs/` 下的 worker 日志，tail 采用 hermes 的
  断行安全语义；P132 新增 `task_runs` 尝试历史表（hermes `Run` 生命
  周期：认领时开 run，携带认领锁/TTL 与运行时限，心跳与 spawn 出的
  worker pid 同步写入，按 hermes outcome 语义关闭 —— done/block/
  reclaim/过期回收/超时分别对应 completed/blocked/reclaimed/
  timed_out，CLI 直接完成未认领任务与调度器 spawn 失败则合成瞬时
  run；重新认领时把残留活跃 run 恢复为 `reclaimed`）、
  `kanban runs [--json] [--state-type status|outcome --state-name V]`
  CLI（hermes 表格格式）与 `latest_run` / `latest_summary` 存储
  辅助方法；P133 接通网关调度器的 triage 自动分解路径（hermes
  `_auto_decompose_tick`）：每个 tick 从配置实时重读
  `[kanban] auto_decompose`（默认开）/ `auto_decompose_per_tick`
  （默认 3），翻转开关在下一 tick 即停止失控扇出、无需重启网关
  （hermes #49638 故障安全语义 —— 配置读取失败则本轮跳过），
  随后在 dispatch 扇出之前经辅助 LLM 分解至多 N 个 triage 任务，
  成功记 info、无操作跳过记 debug；P134 补齐 hermes kanban CLI 的
  最后几块：`kanban context`（完整移植 `build_worker_context` ——
  带上限的正文/附件、历史尝试的 run 摘要与 metadata、已完成父任务
  的交接结果与相对时间陈旧度提示、assignee 跨任务角色历史、带上限
  的评论区；`kanban_show` 工具同步返回 `worker_context`，spawn 出的
  worker 无需额外往返即可读取）、`kanban repair`（integrity_check +
  内容寻址隔离备份 + 仅索引损坏的 REINDEX 自动修复，其余情况保守
  失败）、`kanban assignees`（配置名册与看板 assignee 合并、按状态
  计数）、`kanban daemon`（hermes 已弃用的存根，指向网关；`--force`
  保留独立循环）以及 `ls`/`new` 可见别名；P135 接通通知投递：
  网关运行 kanban notifier 循环（hermes kanban_watchers 通知器，
  5 秒一拍），轮询 `kanban_notify_subs`，领取未送达的终态事件
  （completed/blocked/gave_up/crashed/timed_out/status，其中
  archived/unblocked 只推游标不发声，避免堵塞后续事件），按 hermes
  消息格式渲染（✔ 完成 + 交接首行、⏸ 阻塞 + 原因、⏱ 超时、
  ✖ 崩溃/放弃、🔄 状态变更，带 @assignee 与 [board] 标签），经已
  注册的平台发送器投递，投递后推进每个订阅的游标；订阅在任务
  崩溃/重试周期中保留，仅当任务真正 done/archived 时移除（去重靠
  游标）。与 hermes 的范围差异：无按 profile 的适配器归属（单一
  共享存储）、无线程路由与死聊天清理（PlatformSender 无失败通
  道），发送视为已送达；P136 移植统一失败记账与熔断器（hermes
  `_record_task_failure`）：任务新增 `consecutive_failures` /
  `last_failure_error` / `max_retries` 列，每次 spawn 失败与超时
  尝试都消耗重试预算，达到阈值（按任务 `max_retries` > 调度器
  limit > 默认 2）即 ready→blocked 并发出 `gave_up` 事件（payload
  含 failures / effective_limit / limit_source / trigger_outcome），
  计数器在任务完成与主动 unblock 时清零（hermes 重新起步策略）。
  CLI：`kanban create --max-retries N`（校验 >= 1，与 hermes 一
  致），网关创建 API 接受同一字段；P137 补齐调度器的 worker 健康
  检测（hermes `detect_crashed_workers` + `detect_stale_running`）：
  每 tick 立即回收 worker pid 已死亡的 running 任务（30 秒启动
  宽限期，`ULNCLAW_KANBAN_CRASH_GRACE_SECONDS` 可覆盖；发
  `crashed` 事件、run 以 `crashed` 关闭、计入熔断预算），以及运行
  超过 `[kanban] stale_timeout_seconds`（hermes
  `dispatch_stale_timeout_seconds`，默认 14400，0 关闭，网关循环
  实时重读）且心跳缺失或超过 1 小时的任务（worker 先 SIGTERM 后
  SIGKILL，发 `stale` 事件、run 以 `stale` 关闭，按 hermes 策略
  不计为失败）；两者分别进入 `DispatchResult.stale` / `.crashed`；
  P138 加固内嵌调度器的运维安全（hermes gateway 循环）：排他
  `flock` 单例锁（`<home>/kanban/dispatcher.lock`）保证全机只有
  一个网关进程在调度 —— 第二个网关记录竞争日志、继续提供 HTTP
  但不调度（防配置漂移与重启竞争的兜底）—— 并新增调度器卡死
  健康遥测：ready 队列连续 6 拍非空却零 spawn 时告警（300 秒
  节流）；P139 移植 hermes 的按任务工作区：任务新增
  `workspace_kind`（默认 `scratch` / `worktree` / `dir`）、
  `workspace_path`、`branch_name` 三列；`kanban create --workspace
  scratch|worktree|worktree:<path>|dir:<path>` 与 `--branch <名>`
  （仅 worktree 可用，校验文案与 hermes 一致），网关创建 API 接受
  相同字段；调度器在 spawn 之前解析工作区（hermes
  `resolve_workspace` / `_resolve_worktree_workspace`）：scratch 目录
  位于 `<home>/kanban/workspaces/<id>`，`dir:` 路径必须为绝对路径
  （防混淆代理人穿越，沿用 hermes 威胁模型），worktree 以看板
  `default_workdir` 为锚（未配置时回退调度器 CWD，保留 P139 之前的
  行为；hermes 则直接报错），在 `<repo>/.worktrees/<task-id>` 物化
  分支 `wt/<task-id>`（或 `--branch` 指定），被兄弟任务占用的检出
  会自动改用同仓库下的新树；解析出的路径与分支持久化到任务行供
  重试复用，解析失败按 `workspace:` 前缀计入 spawn 失败熔断，
  `kanban claim` 认领时解析并打印工作区（hermes `_cmd_claim`），
  `[kanban] worktrees=true` 对未显式指定 `--workspace` 的任务保持
  原语义，decompose 子任务继承根任务的工作区类型/路径（worktree
  子任务各自独占新树，hermes 兄弟任务策略）；P140 补齐重生守卫
  与时长语法：`kanban create --max-runtime` 接受
  `30s`/`5m`/`2h`/`1d` 与纯秒数（hermes `_parse_duration`）；
  调度器对立即重试无益的就绪任务延后重生（hermes
  `check_respawn_guard`）—— `rate_limit_cooldown`（最近一次 run 以
  `rate_limited` 结束且仍在冷却期内，
  `ULNCLAW_KANBAN_RATE_LIMIT_COOLDOWN_SECONDS` 默认 300，0 关闭）、
  `blocker_auth`（最近失败命中配额/鉴权模式）、`recent_success`
  （1 小时内有已完成 run 且其后无主动重新入队）、`active_pr`
  （24 小时内评论中出现 GitHub PR 链接）；被守卫的任务保持
  ready，每次延后都会记录 `respawn_guarded` 事件，网关 dispatch
  API 一并返回；P141 移植唤醒路由：任务新增 `session_id` 列，由
  agent `kanban_create` 工具写入（网关创建 API 亦接受该字段）；
  订阅任务进入可唤醒终态事件（`completed` / `gave_up` /
  `crashed` / `timed_out` / `blocked` —— hermes `_WAKE_KINDS`）
  时，通知器以 hermes 格式的唤醒文案（`[kanban] Task <id>
  <status>. …`）自 POST 网关自身的 `/v1/chat/completions` 并携带
  `X-Ulnclaw-Session-Id`，恢复创建者会话（hermes
  `_self_post_chat_completion`：通配绑定走回环、配置了密钥则带
  bearer、单轮上限 600 秒、429/瞬时错误按 2/5/10 秒退避重试、
  其余 HTTP 错误快速失败）；唤醒在文本通知之后尽力异步执行，
  不阻塞其他订阅；P142 移植类型化阻塞（hermes
  `block_task(kind=…)`）：`kanban block --kind dependency` 把任务
  停进 `todo`（`dependency_wait` 事件），由父任务门控 +
  `recompute_ready` 在父任务完成后自动晋升 —— 无需人工与定时
  解锁；`needs_input` / `capability` / `transient` / 未类型化进入
  `blocked` 并持久化 `block_kind` 与 `block_recurrences`，解锁循环
  熔断器在同一原因于解锁后再次阻塞达到
  `BLOCK_RECURRENCE_LIMIT`（2）次时把任务改路由到 `triage`
  （`block_loop_detected` 事件）—— 复发计数刻意跨越 unblock 保留、
  仅在任务完成时清零；`unblock_task` 现在按未完成父任务重新门控
  （父任务未了则 blocked → `todo`），与 hermes 的不变式修复一致；
  agent `kanban_block` 工具与网关 block API 同步接受 kind；P143 补齐生命周期 CLI 面：批量 `kanban
  done/block/schedule/unblock/promote/archive`（多个 id，hermes
  `task_ids` + `--ids`），`kanban done --summary/--metadata` 把结构化
  交接（完整 summary + JSON 事实）写入收尾 run，`completed` 事件则
  携带 summary 首行（400 字符上限）供通知器渲染，`kanban archive
  --rm` 彻底清除已归档任务及其全部关联行（护栏：仅 archived 可
  删除），`kanban unblock --reason` 先记评论再解锁，`kanban promote
  --dry-run/--json` 由无副作用的 `validate_promote` 支撑，`kanban
  watch --tenant`；archive 现在同时把进行中的 run 以 reclaimed 收
  尾，并立即晋升那些仅被已归档父任务挡住的子任务
  （`recompute_ready` 按 hermes 语义把 archived 父任务视同完成）。
  P144 补齐完成态恢复：`kanban edit --result/--summary/--metadata`
  可改写已 done 任务的交接（result 文本 + 最近一次 completed run 的
  summary/metadata，缺少 run 行时自动合成；发出 `edited` 事件），
  terminal kanban 工具新增 `summary` + `metadata` 参数供 worker 交付
  结构化事实，block 操作先写 `BLOCKED: <reason>` 评论再落锁
  （hermes `_cmd_block` 对齐）。P145 把 `recompute_ready` 扩展到
  blocked 列：父任务全部 done/archived 的阻塞任务自动恢复为 ready
  （保留 `consecutive_failures`，发出 `promoted` 事件），除非阻塞是
  粘性的 —— 最近一次 blocked/unblocked 事件是 worker/运维主动发起的
  `blocked`（#28712）—— 或失败计数已达到有效上限（任务 `max_retries`
  > 调度器 `failure_limit` > 默认 2，#35072）；调度器经
  `dispatch_once` 透传自身配置的上限。P146 补上不可 spawn 门控与健康
  探针：`dispatch_once` 接受配置的 profile 集合，把 assignee 不在集合
  内的就绪任务归入 `skipped_nonspawnable`（只能经 claim 拉取的控制面
  通道，绝不自动 spawn —— hermes #kanban-dispatcher-crash-loop）；
  网关调度器的 stuck 警告改由 `has_spawnable_ready` 判定，就绪队列里
  只有通道任务时视为"正常空闲"，仅在确有可 spawn 工作（未指派或已配
  置 profile 的任务）等待时才告警。P147 移植完成工件：`kanban done
  --artifact <path>`（可重复）、agent `kanban_done` 工具（`artifacts`
  数组）与网关 complete API 会把托管 scratch 工作区内的文件先行暂存
  到 `<home>/kanban/attachments/<task>/`（25 MiB 上限，缺失/超限的声
  明直接让完成失败并回滚），登记为 `artifact` 附件并发出 `attached`
  事件，同时合并 summary/result 文本中提到的绝对交付物路径，最终路
  径随 `completed` 事件与 run metadata 下发（hermes
  `kanban_complete(artifacts=[...])`、
  `_persist_scratch_completion_artifacts`、
  `_merge_completion_prose_artifacts`）。P148 移植 review 列：
  `review` 加入状态集（🔍）；worker 开 PR 后调用 `kanban review <id>
  [--reason]` / `kanban_review` 工具（running → review，收尾 worker
  run，发出 `review_requested` 事件）；`dispatch_once` 新增共享
  max_spawn 上限的 review 循环 —— 未指派的 review 任务归入
  `skipped_unassigned`，未知 assignee 归入 `skipped_nonspawnable`，
  认领时开新 run 且不再复查父任务依赖（`claim_review_task`），
  `<home>/skills/` 装有 `sdlc-review` 技能时强制加载；
  `has_spawnable_review` 并入网关健康探针。P149 增加按 profile 的
  并发上限：`[kanban] max_in_progress_per_profile`（hermes #21582）
  即使全局仍有余量，也拒绝为已达在飞上限的 assignee 再生 worker ——
  计数每 tick 从 running 列播种、dry-run 的拟 spawn 同样计数；被跳过
  的任务归入 `skipped_per_profile_capped`（CLI 行 + dispatch JSON）。
  P150 移植反幻觉完成门：`kanban done --created-card <id>`（可重复；
  agent `created_cards` 数组与网关 complete API 同步）逐一核验声明的
  卡片 —— 必须存在，且由该 worker 的 profile 创建、以 worker 任务 id
  作为 created_by、或已挂为 worker 任务的子任务。幻影 id 发出
  `completion_blocked_hallucination` 事件并在零改动下阻断完成
  （hermes `HallucinatedCardsError`）；核验通过的 id 随 `completed`
  事件下发，summary/result 文本中无法解析的 `t_<hex>` 引用在成功完成
  后以 `suspected_hallucinated_references` 事件提示（仅告警，hermes
  `_scan_prose_for_phantom_ids`）。P151 增加按 tick 的调度锁
  （#35240）：每次 `dispatch_once` 都在 `<kanban.db>.dispatch.lock`
  的非阻塞 `flock` 下进行；失败的调度器（例如逃出服务重启的孤儿进
  程）返回 `skipped_locked = true` 且零数据库写入，下一间隔再试 ——
  CLI（`dispatch: skipped …`）与 dispatch API JSON 均可见。P152 增
  加 worker 日志轮转：`kanban/worker-logs/` 下的按任务日志在达到
  `[kanban] worker_log_rotate_bytes`（默认 2 MiB）时轮转，保留一份
  `.log.1` 备份代，同一代内追加写入 —— 重生 attempt 不再截断先前输
  出（hermes `worker_log_rotation_config`）。P153 封死过期 worker
  竞态：调度改为先认领再 spawn（hermes 顺序），spawn 时 run 行已存
  在；worker 携带 `ULNCLAW_KANBAN_RUN_ID`（hermes
  `HERMES_KANBAN_RUN_ID`），其完成/阻塞以 `expected_run_id` 提交 ——
  原子的 `current_run_id` 守卫拒绝已被回收的 attempt，绝不覆盖新
  attempt（CLI Done/Block、`kanban_complete`/`kanban_block` 工具与
  网关 complete/block API 全链路透传）。已认领 attempt 的 spawn/工作
  区失败现在收尾 run、释放认领回 ready 并计入失败（hermes
  `_record_spawn_failure`）。
- 消息平台：媒体以缓存路径引用交付，agent 用 vision_analyze/
  video_analyze/read_file 检视（hermes 的原生多模态用户轮注入未移植）；
  语音消息经 `[stt]` 管道转写，但内置 `local` faster-whisper provider
  在静态二进制中需以 `stt.local.command` 或云 provider 替代；无内联键盘；
  移植了 hermes 二十一个平台适配器中的十一个（微信/QQ/元宝以原生适配器加入；WhatsApp Cloud + MS-Graph
  接入与通用 webhook 平台均经网关 webhook 路由）；配对与 hermes 相同按
  发送者 id 生效，但配置式白名单仍以聊天/频道 id 为粒度（认证门控 =
  白名单 OR 已批准配对）。
- OAuth/同步：流程与提供方无关（任意 RFC 8628 端点），不绑定 Nous 门户；
  同步只搬运技能包（无组织提案/审批工作流、无订阅门控）。

## 完成状态

agent 核心已与 hermes-agent v2026.8.3 对齐：全部核心工具、完整 `sessions`
面（`list/show/search/export/recap/recover/prune/archive/stats/delete/
rename/optimize/repair/browse`）、启动恢复（`--resume`/`--continue`，
一会话一记录的连续性）、CDP 浏览器客户端 + 接入层 + Camofox 后端、HTTP
网关（含 `/v1/browser/*` 实时端点控制与 profile 多路复用）、技能/捆绑/
记忆/目标/检查点/定时任务/用量分析/doctor、外部秘密源（command 助手 /
Bitwarden / 1Password）、computer-use（cua-driver）、子进程插件系统、
消息平台网关（Telegram/Discord/Slack）、OAuth 设备流登录 + 技能同步、Tauri 桌面 GUI（`desktop/`，取代 hermes 的 Electron 应用）及其余 CLI 均已移植。`sessions`
面仅有意省略：`optimize-storage`（ulnclaw 自始即采用紧凑的外联内容 FTS
布局，无旧布局可迁移）与 `-c <会话名>` 标题查找（`--continue` 不带值）。

有意不移植（超出本地 agent 范围的 hermes 面）：Electron
桌面应用（ulnclaw 改以 Tauri `desktop/` 外壳交付；Electron 专属的
kanban/仪表盘挂件仍未移植）、插件/hook/
egress 体系（含其后的云浏览器 provider）、Python 插件导入/entry-point 包及其携带的 provider 注册，以及小型桌面
UX 命令（clipboard、focus_view、prompt_stash、uninstall）。
