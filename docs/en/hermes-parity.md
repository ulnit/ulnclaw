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
| `browser_*` (12 tools) | 🟡 gated | registered with faithful schemas; require a CDP backend (`ULNCLAW_BROWSER_CDP`) — client integration pending |
| `computer_use` | 🟡 gated | requires a computer-use driver (hermes: cua-driver) |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 gated | registered, gated on platform credentials; backends pending |
| `x_search`, `video_analyze`, `video_generate`, `bfl_flux3_*` | ⬜ deferred | provider-specific (xAI/BFL); add when credentials are available |

## Feature parity

| hermes feature | ulnclaw | Notes |
|---|---|---|
| Agent loop with tool calling | ✅ | iteration budget, usage accounting, step callbacks |
| SQLite state store (`hermes_state.py`) | ✅ | sessions/messages/system_prompts/state_meta/async_delegations schema, FTS5 with LIKE fallback, lineage (parent sessions) |
| Context compression (`conversation_compression.py`) | ✅ | budget-triggered, middle-turn summarization via secondary model call, keeps system prompt + first user message + recent tail |
| Approval system (`approval.py`) | ✅ core | command normalization (backslash-joins, `${IFS}`, comment strip), hardline floor (block), recoverable-costly (confirm) |
| Threat-pattern scanning (`threat_patterns.py`) | ✅ core | advisory injection scan for tool results re-entering context |
| Toolsets (`toolsets.py`) | ✅ | all 33 toolset definitions incl. composition (`includes`), `coding` default |
| Tool registry (`registry.py`) | ✅ | check_fn gating, toolset grouping, max result size truncation |
| Provider abstraction (`runtime_provider.py`) | ✅ | OpenAI-compatible (OpenAI/OpenRouter/DashScope/Ollama/llama.cpp), keyless local providers |
| Config (`config.yaml`) | ✅ | `config.toml` + `.env` file, profiles, env precedence |
| Skills system | ✅ | discovery, frontmatter, linked files |
| Memory system | ✅ | MEMORY.md/USER.md with prompt injection |
| Cron scheduler | ✅ | job store + schedule parsing + poll loop (`cron::run_scheduler`) |
| MCP client (`mcp_tool.py`) | ✅ core | stdio JSON-RPC: initialize/tools/list/tools/call; `[[mcp.servers]]` config; tools registered as `mcp__<server>__<tool>` |
| CLI (`hermes_cli/`) | ✅ core | chat REPL with slash commands, one-shot `run`, sessions/tools/skills/cron subcommands, `init` |
| Delegation | ✅ | SubAgentRunner trait, depth limit, child sessions |
| Gateway/TUI/web/app surfaces | ⬜ deferred | hermes ships Discord/Telegram/etc. gateways, a TUI and web apps; ulnclaw is a library + CLI today |
| Environments (docker/ssh/modal/daytona/vercel) | ⬜ deferred | terminal runs locally; remote backends pending |
| Checkpoint manager, browser supervisor, computer-use CUA | ⬜ deferred | heavyweight subsystems |

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
└── sandboxes/              execute_code scripts
```

## Known differences

- Approval UX is a terminal y/N prompt (hermes has richer platform-specific flows).
- Browser tools are schema-faithful stubs until a CDP client is integrated.
- Compression uses a char/4 token estimate instead of a tokenizer.
- `patch` fuzzy chain implements the 4 deterministic hermes strategies
  (exact → line-trimmed → whitespace-normalized → indent-flexible); the two
  similarity-based strategies (block_anchor, context_aware) are not ported.
