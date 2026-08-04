# Hermes Agent Parity Matrix (v2026.8.3)

This document tracks how ulnclaw maps to
[hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3).
ulnclaw is a Rust re-implementation of the hermes agent engine: same tool
surface, same storage layout, same configuration semantics — native
performance and a single static binary.

## Tool parity

| hermes tool | ulnclaw | Notes |
|---|---|---|
| `terminal`, `process` | ✅ full | foreground/background, timeouts, workdir tracking, background session registry (list/log/wait/kill) |
| `read_file`, `write_file`, `patch`, `search_files` | ✅ full | line-numbered reads with `next_offset` pagination, fuzzy replace (whitespace/indent tolerant), V4A multi-file patches, unified diffs, ripgrep-style search |
| `web_search`, `web_extract` | ✅ full | pluggable backends: Tavily / Brave / SearXNG / built-in DuckDuckGo; HTML→text extraction |
| `memory` | ✅ full | `MEMORY.md` + `USER.md`, atomic batched `operations`, char limits (2200/1375), injected into every system prompt |
| `todo` | ✅ full | session task list, merge mode, single `in_progress` enforcement |
| `session_search` | ✅ full | SQLite FTS5 discovery + scroll shapes, session lineage |
| `clarify` | ✅ full | single/multi-select or open-ended via frontend callback |
| `skills_list`, `skill_view`, `skill_manage` | ✅ full | SKILL.md frontmatter, linked files (references/templates/scripts), path-traversal guard |
| `delegate_task` | ✅ full | parallel sub-agents, depth limit, isolated context, child sessions |
| `execute_code` | ✅ full | python3 subprocess sandbox, 120s cap |
| `cronjob` | ✅ full | create/list/update/pause/resume/remove/run; `30m` / `every 2h` / `0 9 * * *` / ISO one-shot schedules; SQLite job store; scheduler loop |
| `tool_search` | ✅ full | keyword search over the registered tool catalog |
| `vision_analyze` | ✅ full | routes through the chat provider (`analyze_image`) |
| `image_generate` | ✅ full | OpenAI images API, saves PNG under `<home>/images` |
| `text_to_speech` | ✅ full | OpenAI TTS or custom `ULNCLAW_TTS_ENDPOINT` |
| `ha_list_entities`, `ha_get_state`, `ha_list_services`, `ha_call_service` | ✅ full | Home Assistant REST API, gated on `HASS_URL` + `HASS_TOKEN` |
| `kanban_*` (12 tools) | ✅ full | local SQLite coordination board: create/list/show/complete/block/unblock/comment/heartbeat/link/attach/attach_url/attachments |
| `browser_*` (12 tools) | ✅ full | CDP WebSocket client (`browser` module): endpoint discovery, page session, accessibility snapshots with element refs, click/type/scroll/press/screenshot/evaluate/dialogs; `ULNCLAW_BROWSER_CDP` accepts ws://, http://host:port, or `auto` (supervisor launches a managed headless Chrome/Chromium) |
| `computer_use` | 🟡 gated | requires a computer-use driver (hermes: cua-driver) |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 gated | registered, gated on platform credentials; backends pending |
| `x_search`, `video_analyze`, `video_generate`, `bfl_flux3_*` | ⬜ deferred | provider-specific (xAI/BFL); add when credentials are available |

## Feature parity

| hermes feature | ulnclaw | Notes |
|---|---|---|
| Agent loop with tool calling | ✅ | iteration budget, usage accounting, step callbacks |
| SQLite state store (`hermes_state.py`) | ✅ | sessions/messages/system_prompts/state_meta/async_delegations schema, FTS5 with LIKE fallback, lineage (parent sessions) |
| Context compression (`conversation_compression.py`) | ✅ | budget-triggered, middle-turn summarization via secondary model call, keeps system prompt + first user message + recent tail |
| Approval system (`approval.py`) | ✅ | command normalization (backslash-joins, `${IFS}`, comment strip), hardline floor (block), recoverable-costly (confirm); REPL y/N prompt; gateway run approvals (`POST /v1/runs/:id/approval`, once/session/always/deny, SSE `approval.request`), fail-closed `[approvals] timeout` (default 300s), `always` grants persisted across restarts |
| Threat-pattern scanning (`threat_patterns.py`) | ✅ core | advisory injection scan for tool results re-entering context |
| Toolsets (`toolsets.py`) | ✅ | all 33 toolset definitions incl. composition (`includes`), `coding` default |
| Tool registry (`registry.py`) | ✅ | check_fn gating, toolset grouping, max result size truncation |
| Provider abstraction (`runtime_provider.py`) | ✅ | OpenAI-compatible (OpenAI/OpenRouter/DashScope/Ollama/llama.cpp), native Anthropic Messages transport (`anthropic_messages`: system param, tool_use/tool_result blocks, SSE streaming, max_tokens ceilings, OAuth bearer), keyless local providers |
| Config (`config.yaml`) | ✅ | `config.toml` + `.env` file, profiles, env precedence |
| Skills system | ✅ | discovery, frontmatter, linked files |
| Memory system | ✅ | MEMORY.md/USER.md with prompt injection |
| Cron scheduler | ✅ | job store + schedule parsing + poll loop (`cron::run_scheduler`) |
| MCP client (`mcp_tool.py`) | ✅ core | stdio JSON-RPC: initialize/tools/list/tools/call; `[[mcp.servers]]` config; tools registered as `mcp__<server>__<tool>` |
| CLI (`hermes_cli/`) | ✅ core | chat REPL with slash commands (incl. `/rollback [N|hash] [file]`, `/rollback diff <N>`, `/diff` checkpoint commands, `/recap`), one-shot `run`, sessions/tools/skills/cron/checkpoints subcommands (incl. `sessions export --format md\|html` — SHA256-verified Markdown or standalone HTML + manifest — and `sessions recap`), `init` |
| Delegation | ✅ | SubAgentRunner trait, depth limit, child sessions |
| HTTP gateway (`gateway/platforms/api_server.py`) | ✅ core | `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` (session continuity via `X-Ulnclaw-Session-Id`, `stream: true` SSE token streaming with `hermes.tool.progress` events), `/v1/responses` (stateful via `previous_response_id`, `stream: true` Responses-API SSE events), `/v1/models`, `/api/model/options`, `/v1/capabilities`, `/v1/runs` (async runs + SSE events + stop + approval), `/api/sessions` CRUD + chat + chat/stream + `PATCH` (title/end_reason) + `fork` + per-session model lock (enforced on every turn) + `recap`, `/api/jobs` cron HTTP API (CRUD + pause/resume/run), `/v1/skills`, `/v1/toolsets`, bearer-token auth |
| Messaging platforms (Telegram/WhatsApp/QQ/…) | ⬜ deferred | hermes' platform adapters are not ported; the HTTP gateway covers OpenAI-compatible frontends |
| TUI/web/app surfaces | ⬜ deferred | hermes ships a TUI and web apps; ulnclaw is a library + CLI + HTTP gateway today |
| Environments (`tools/environments/`) | ✅ core | `terminal` backends: local (default), docker (`ensure_docker_container` inspect→run), ssh (BatchMode, identity file); `[terminal] backend/container/image/ssh_host/...`; modal/daytona/vercel deferred |
| Checkpoint manager (`checkpoint_manager.py`) | ✅ | v2 shared shadow git store (`<home>/checkpoints/store`): per-project refs/indexes, transparent pre-edit snapshots (once per turn before `write_file`/`patch`), list/restore/diff/prune CLI, size caps, oversize-file filter, orphan/stale auto-prune |
| Browser supervisor | ✅ | auto-launches managed headless Chrome/Chromium for `ULNCLAW_BROWSER_CDP=auto` |
| computer-use CUA | ⬜ deferred | requires a computer-use driver |

## Storage layout

```
~/.ulnclaw/                 (ULNCLAW_HOME override supported; HERMES_HOME honored for migration)
├── config.toml             main configuration
├── .env                    KEY=VALUE secrets (checked after process env)
├── state.db                SQLite: sessions, messages, cron_jobs, meta (+FTS5)
├── kanban.db               kanban board
├── memory/MEMORY.md        agent memory
├── memory/USER.md          user profile
├── skills/<name>/SKILL.md  skills
├── sessions/*.todos.json   per-session todo lists
├── images/  audio/         generated artifacts
├── sandboxes/              execute_code scripts
├── approvals.json          persisted "always" approval grants
└── checkpoints/store/      shared shadow git store (per-project refs/indexes)
```

## Known differences

- Approval UX is a terminal y/N prompt on the CLI (hermes has richer
  platform-specific flows); the gateway exposes run approvals over HTTP with
  once/session/always/deny semantics.  Chat-completions requests have no run
  context and auto-deny confirm-tier commands by design.  hermes' Smart-DENY
  (LLM-assisted) verdicts and cron approval modes are not ported; unattended
  runs fail closed.
- Browser supervisor launches a local Chrome/Chromium directly; hermes drives
  an external `agent-browser` daemon (cloud browser providers are not ported).
- The gateway implements the api_server platform subset; profile multiplexing
  (`/p/<profile>/...`) is not ported.  The jobs API delivers locally only
  (`deliver="local"`); hermes' external delivery targets and the NAS/Chronos
  fire webhook (`/api/cron/fire`) are not ported.
- `/api/model/options` returns the single configured provider row; hermes'
  multi-provider inventory (live catalog probing, pricing, capabilities,
  featured models via models.dev) is not ported.
- Compression uses a char/4 token estimate instead of a tokenizer.
- `patch` fuzzy chain implements all 9 hermes strategies; similarity is an
  LCS-based ratio (difflib.SequenceMatcher stand-in), so edge-case thresholds
  can differ slightly from CPython's matcher.
- Environments cover local/docker/ssh; hermes' modal/daytona/vercel backends
  and their credential flows are not ported.
- Checkpoints skip hermes' legacy pre-v2 store migration (fresh stores only)
  and the volume-identity orphan heuristic (workdir existence is used).
