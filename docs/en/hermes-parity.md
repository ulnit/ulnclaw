# Hermes Agent Parity Matrix (v2026.8.3)

This document tracks how ulnclaw maps to
[hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3).
ulnclaw is a Rust re-implementation of the hermes agent engine: same tool
surface, same storage layout, same configuration semantics — native
performance and a single static binary.

## Tool parity

| hermes tool | ulnclaw | Notes |
|---|---|---|
| `terminal`, `process` | ✅ full | foreground/background, timeouts, workdir tracking, background session registry (list/log/wait/kill), failure intelligence (benign exit-code semantics + output-pattern recovery hints) |
| Tool-output limits (`tool_output_limits.py`) | ✅ | `[tool_output] max_bytes/max_lines/max_line_length` tune terminal output head+tail cap (default 100k chars), read_file pagination cap (2000 lines), and per-line clamp with `... [truncated]` marker (2000 chars); non-positive values coerce to defaults; behaviour-preserving when unset |
| Terminal failure hints (`terminal_hints.py`, `_interpret_exit_code`) | ✅ | `exit_code_meaning` for benign non-zero exits (grep/rg/diff/find/test/curl/git semantics, last pipeline/chain segment wins, `VAR=val` prefixes skipped); at most one `hint` per failed command from an ordered output-pattern scan (gh JSON-field drift, merge conflicts, command not found — python/pip special-cased, ModuleNotFoundError/ImportError, "already exists", gh rate limits, permission denied) plus exit-code-only hints 124/126/137; bounded 4000-char scan window, first match wins |
| Secret redaction (`agent/redact.py`) | ✅ core | terminal output (foreground + process log/wait) and read_file content pass through the redactor: ~55 vendor-prefix tokens (sk-/ghp_/glpat-/AKIA/xox…/JWT/private keys/DB connstrings/auth & x-api-key headers), ENV-assignment masking for env-dump commands, JSON/YAML secret fields otherwise; file-read content gets non-reusable `«redacted:prefix…»` sentinels so agents can't write truncated keys back (hermes #35519); web-URL query-param redaction stays opt-in; full Smart-DENY log pipeline and profile secret scopes not ported |
| ANSI stripping (`ansi_strip.py`) | ✅ | full ECMA-48 coverage (CSI incl. private-mode/colon params/intermediates, OSC with BEL/ST terminators, DCS/SOS/PM/APC, nF and single-byte escapes, 8-bit C1) strips terminal + execute_code output before it reaches the model; `sanitize_display_text` additionally drops bare control chars and normalizes CR for safe terminal re-rendering |
| Binary extension guard (`binary_extensions.py`) | ✅ | `read_file` rejects ~80 binary extensions by pure string check (no I/O), pointing at vision_analyze/terminal; `.pdf` stays readable (text-based) |
| `read_file`, `write_file`, `patch`, `search_files` | ✅ full | line-numbered reads with `next_offset` pagination, fuzzy replace (whitespace/indent tolerant), V4A multi-file patches, unified diffs, ripgrep-style search |
| `web_search`, `web_extract` | ✅ full | pluggable backends: Tavily / Brave / SearXNG / built-in DuckDuckGo; HTML→text extraction |
| URL safety / SSRF guard (`tools/url_safety.py`) | ✅ core | `url_safety` module: blocks web fetches to private/internal addresses (loopback, RFC1918, link-local, CGNAT 100.64/10, benchmark 198.18/15, ULA, IPv4-mapped IPv6); cloud metadata endpoints (169.254.169.254, metadata.google.internal, ECS task metadata…) are **always** blocked; wired into `web_extract` (per-URL check + redirect re-validation via reqwest policy + credential-bearing URL refusal: token-prefix and sensitive-query-param blocks), opt-out `[security] allow_private_urls` / `ULNCLAW_ALLOW_PRIVATE_URLS`; fail-closed on DNS errors with a proxy carve-out (hermes semantics) |
| `memory` | ✅ full | `MEMORY.md` + `USER.md`, atomic batched `operations`, char limits (2200/1375), injected into every system prompt |
| `todo` | ✅ full | session task list, merge mode, single `in_progress` enforcement |
| `session_search` | ✅ full | SQLite FTS5 discovery + scroll shapes, session lineage |
| `clarify` | ✅ full | single/multi-select or open-ended via frontend callback |
| `skills_list`, `skill_view`, `skill_manage` | ✅ full | SKILL.md frontmatter, linked files (references/templates/scripts), path-traversal guard |
| Blueprints (`tools/blueprints.py`) | ✅ core | skills that declare `metadata.hermes.blueprint.schedule` in frontmatter become schedulable: `skills blueprints` (list), `skills schedule <name>` (creates the `blueprint:<skill>` cron job with the skill attached), `skills unschedule <name>`; malformed blueprint blocks error loudly; `skills list` marks blueprints with the schedule. The hermes suggestion queue + `export_blueprint` publish path are not ported (explicit commands instead) |
| Skills guard (`tools/skills_guard.py`) | ✅ core | `skills scan <name> [--source <repo>] [--json] [--force]`: static scanner `skills-guard-v1` over SKILL.md + linked files — 119 threat patterns (exfiltration/destructive/persistence/supply-chain/prompt-injection), invisible-unicode detection, structural limits (50 files / 1 MB / 256 KB per file, symlink-escape + exec-bit checks), trust levels (builtin / agent-created / trusted repos incl. prefix aliases / community), verdict policy (critical→dangerous, high→caution; community+caution blocked, dangerous blocked even for trusted, `--force` only overrides caution for non-community) |
| `delegate_task` | ✅ full | parallel sub-agents, depth limit, isolated context, child sessions; hermes v2026.8.3 background semantics: top-level delegations dispatch fire-and-forget (`mode: background`, delegation_id, live transcripts under `cache/delegation/live/<id>/task-N.log`) and ONE consolidated result re-enters the conversation when all children finish (REPL drain + gateway session-chat drain); orchestrator children (depth > 0) stay synchronous; one-shot/stateless sessions force synchronous execution with a note (`tools/async_delegation.py` port incl. durable sqlite registry: dispatches + consolidated results persist to `async_delegations`, startup recovery turns rows still `running` after a crash into terminal `unknown` outcomes, drains claim undelivered rows through the durable delivery-claim lifecycle — per-claim `delivery_attempts`, 300s stale-claim takeover, completions whose origin session is gone converge to terminal `dropped` after 8 attempts, successful injection marks `delivered` under the claim token); `GET /v1/delegations` + `/v1/delegations/:id` registry endpoints (ulnclaw ops extension) |
| `execute_code` | ✅ full | python3 subprocess sandbox, 120s cap |
| `cronjob` | ✅ full | create/list/update/pause/resume/remove/run; `30m` / `every 2h` / `0 9 * * *` / ISO one-shot schedules; SQLite job store; scheduler loop — the gateway auto-dispatches due jobs every 30s as tracked cron runs (cron approval scope, outcome recorded on the job row) and `ulnclaw cron run <id>` executes one immediately from the CLI |
| `tool_search` | ✅ full | keyword search over the registered tool catalog |
| `vision_analyze` | ✅ full | routes through the chat provider (`analyze_image`), `[auxiliary.vision]` provider/model override |
| `image_generate` | ✅ full | OpenAI images API, saves PNG under `<home>/images` |
| `text_to_speech` | ✅ full | OpenAI TTS or custom `ULNCLAW_TTS_ENDPOINT` |
| `ha_list_entities`, `ha_get_state`, `ha_list_services`, `ha_call_service` | ✅ full | Home Assistant REST API, gated on `HASS_URL` + `HASS_TOKEN` |
| `kanban_*` (12 tools) | ✅ full | local SQLite coordination board: create/list/show/complete/block/unblock/comment/heartbeat/link/attach/attach_url/attachments |
| `browser_*` (12 tools) | ✅ full | CDP WebSocket client (`browser` module): endpoint discovery, page session, accessibility snapshots with element refs, click/type/scroll/press/screenshot/evaluate/dialogs; `ULNCLAW_BROWSER_CDP` accepts ws://, http://host:port, or `auto` (supervisor launches a managed headless Chrome/Chromium); hermes SSRF guards ported (`browser/guard.rs`): sensitive-query + cloud-metadata floor unconditional, private-address guard for non-local endpoints or containerized terminals, post-redirect recheck, JS URL-literal screening for console/eval, raw-CDP allowlist on private pages; browser outputs force-redacted; live endpoint override via REPL `/browser connect` and gateway `/v1/browser/connect|disconnect|status`; Camofox REST backend via `CAMOFOX_URL` (other cloud browser providers remain not ported) |
| `close_terminal`, `read_terminal`, `focus_pane`, `open_preview` | ✅ core | desktop GUI affordances (hermes `close_terminal_tool.py` / `read_terminal_tool.py` / `focus_pane_tool.py` / `open_preview_tool.py`): registered only under `ULNCLAW_DESKTOP=1`, routed through the `desktop` bridge — a host app installs an emitter (`ulnclaw::desktop::set_emitter`) that receives `(ui_session_id, event, payload)` events (`terminal.close`, `pane.reveal`, `preview.open`) plus a blocking `read_terminal` callback; without a wired host they report "desktop only", never kill processes, and normalize bare domains (`www.cnn.com` → https, `localhost:3000` → http); `react_to_message` (hermes `react_to_message_tool.py` port): agent emoji tapbacks — one reaction per author, same-emoji toggles off, defaults to the latest user message (`messages_back` steps earlier, `message_row_id` targets exactly), persisted in `messages.display_metadata` and painted live via the `message.reaction` bridge event; gated on `ULNCLAW_DESKTOP=1` **and** `[display] message_reactions` |
| `computer_use` | 🟡 gated | requires a computer-use driver (hermes: cua-driver) |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 gated | registered, gated on platform credentials; backends pending |
| `x_search` | 🟡 gated | full port of hermes `x_search_tool.py`: xAI Responses-API `x_search` server tool with handle allow/exclude filters (max 10, `@` stripped), strict client-side date-range validation (YYYY-MM-DD, no inverted/pure-future windows), `enable_image_understanding` / `enable_video_understanding`, retry-with-backoff on 5xx/transient failures, `degraded`/`degraded_reason` markers when filters yield no citations, `[x_search]` config (model / reasoning_effort / timeout_seconds / retries); registered only with `XAI_API_KEY` **and** the opt-in `x_search` toolset enabled (hermes parity — SuperGrok OAuth path not ported) |
| `video_analyze` | ✅ core | full port of hermes `vision_tools.video_analyze_tool`: local file / `file://` / HTTP(S) sources (remote downloads gated by the SSRF guard, cached under `cache/video/temp_video_files/` and cleaned up), extension→mime table (mp4/webm/mov/avi/mkv/mpeg/mpg), 20 MB warn + 50 MB base64 hard cap, inline `video_url` data-URL payload, `[auxiliary.vision]` routing with main-provider fallback, one retry on empty responses; opt-in `video` toolset (hermes parity) — requires a provider that accepts video |
| `video_generate`, `bfl_flux3_*` | ⭜ deferred | provider plugin registry (xAI/BFL/Pixverse/…) — BFL tools run through the Nous gateway; add when credentials are available |

## Feature parity

| hermes feature | ulnclaw | Notes |
|---|---|---|
| Agent loop with tool calling | ✅ | iteration budget, usage accounting, step callbacks |
| SQLite state store (`hermes_state.py`) | ✅ | sessions/messages/system_prompts/state_meta/async_delegations schema, FTS5 with LIKE fallback, lineage (parent sessions) |
| Session recovery (`session_recovery.py`) | ✅ core | `ulnclaw sessions recover <db> [--out FILE]`: offline, non-destructive — source copied (with WAL/SHM/journal sidecars) to a disposable dir, canonical rows copied into a fresh current-schema db with rowid salvage over damaged tables, orphaned messages get reconstructed session rows, FTS rebuilt, integrity-checked, JSON report; never repairs in place or overwrites the active db |
| Environment probe (`tools/env_probe.py`) | ✅ | one deterministic Python-toolchain line in the system prompt when the terminal backend is local: python3/python versions, pip-module availability, `pip`↔`python3` version mismatch, PEP 668 externally-managed marker (uv neutralizes it); silent on healthy machines; process-wide cache built by a single background worker, callers wait ≤10s then fail open; remote backends (docker/ssh) skip the probe; `[agent] environment_probe` toggle (default true) |
| Context compression (`conversation_compression.py`) | ✅ | budget-triggered, middle-turn summarization via secondary model call, keeps system prompt + first user message + recent tail; summary call honors `[auxiliary.compression]` routing |
| Approval system (`approval.py`) | ✅ | command normalization (backslash-joins, `${IFS}`, comment strip), hardline floor (block), recoverable-costly (confirm); REPL y/N prompt; gateway run approvals (`POST /v1/runs/:id/approval`, once/session/always/deny, SSE `approval.request`), fail-closed `[approvals] timeout` (default 300s), `always` grants persisted across restarts; `[approvals] mode = manual|smart|off` — smart mode asks an auxiliary guardian LLM (prompt-injection-hardened prompt, operator `smart_policy` on the trusted channel) and escalates to a human when unsure, `off` auto-approves below the hardline floor; `cron_mode = deny|approve` governs unattended cron runs (deny = fail-closed default) |
| Threat-pattern scanning (`threat_patterns.py`) | ✅ core | advisory injection scan for tool results re-entering context |
| Toolsets (`toolsets.py`) | ✅ | all 33 toolset definitions incl. composition (`includes`), `coding` default |
| Tool registry (`registry.py`) | ✅ | check_fn gating, toolset grouping, max result size truncation |
| Provider abstraction (`runtime_provider.py`) | ✅ | OpenAI-compatible (OpenAI/OpenRouter/DashScope/Ollama/llama.cpp), native Anthropic Messages transport (`anthropic_messages`: system param, tool_use/tool_result blocks, SSE streaming, max_tokens ceilings, OAuth bearer), keyless local providers |
| Provider fallback chain (`fallback_providers`, `try_activate_fallback`) | ✅ core | `[model] fallbacks = ["provider:model", ...]`: on a failed model call the chain advances (lazy per-entry clients, credential fallback to the main key), the activated fallback stays live for the turn, and the next turn restores the primary (hermes `restore_primary_runtime`); delegated/cron children inherit the specs |
| Auxiliary model routing (`auxiliary_client.py`) | ✅ core | `[auxiliary.<task>]` per-task provider/model/base_url/api_key/key_env overrides (`compression`, `vision`); `"auto"`/blank inherits the main runtime; main client reused when nothing is overridden |
| models.dev catalog (`agent/models_dev.py`) | ✅ core | `models_dev.rs`: fetches `https://models.dev/api.json` with a three-tier cache — in-memory (1h TTL, stale served immediately while a background thread refreshes) → disk (`$ULNCLAW_HOME/models_dev_cache.json`, any age) → singleflight network with 5-minute process-wide failure backoff; provider ID mapping + identity fallback, context/capability lookups (case-insensitive, `:cloud`/`-cloud` suffix fallback), agentic catalog filters (noise patterns + Google hidden list), `get_provider_info`/`get_model_info`; `ULNCLAW_MODELS_DEV_URL` mirror override (http(s)/file), `ULNCLAW_MODELS_DEV_CACHE` path override; gateway `/api/model/options` enrichment + `?refresh=true`; CLI `ulnclaw models providers\|list\|info\|refresh` |
| Config (`config.yaml`) | ✅ | `config.toml` + `.env` file, profiles, env precedence |
| Skills system | ✅ | discovery, frontmatter, linked files |
| Memory system | ✅ | MEMORY.md/USER.md with prompt injection |
| Cron scheduler | ✅ | job store + schedule parsing + poll loop (`cron::run_scheduler`) |
| MCP client (`mcp_tool.py`) | ✅ core | stdio JSON-RPC: initialize/tools/list/tools/call; `[[mcp.servers]]` config; tools registered as `mcp__<server>__<tool>`; OSV malware preflight for npx/uvx/pipx launches (`osv_check.py` port: MAL-* advisories block, fail-open, 1h verdict cache, `OSV_ENDPOINT`/`OSV_CHECK_CACHE_TTL` overrides) |
| CLI (`hermes_cli/`) | ✅ core | chat REPL with slash commands (incl. `/rollback [N|hash] [file]`, `/rollback diff <N>`, `/diff` checkpoint commands, `/recap`), one-shot `run`, sessions/tools/skills/cron/checkpoints subcommands (incl. `sessions export --format md\|html` — SHA256-verified Markdown or standalone HTML + manifest —, `sessions recap`, and `sessions recover`), `moa run/list/delete`, `models providers/list/info/refresh` (models.dev catalog), `skills blueprints/schedule/unschedule`, `diff`, `init` |
| Git working diff (`working_diff.py`) | ✅ | `ulnclaw diff [--staged|--all] [--dir PATH] [paths...]` + REPL `/gitdiff [staged|all]`: working/staged/all modes, untracked files folded in via `git diff --no-index` (50-file cap), timeouts; the checkpoint-based REPL `/diff` remains separate |
| Delegation | ✅ | SubAgentRunner trait, depth limit, child sessions |
| Mixture of Agents (`moa_loop.py`, `moa_config.py`) | ✅ core | `[moa.presets.<name>]` reference fan-out + aggregator synthesis (`ulnclaw moa run/list/delete`, REPL `/moa <prompt>`); parallel references, loud/silent degraded policy, all-failed early return, joined-fallback on aggregator failure; the persistent `provider: moa` facade, traces, and privacy filter are not ported |
| HTTP gateway (`gateway/platforms/api_server.py`) | ✅ core | `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` (session continuity via `X-Ulnclaw-Session-Id`, `stream: true` SSE token streaming with `hermes.tool.progress` events), `/v1/responses` (stateful via `previous_response_id`, `stream: true` Responses-API SSE events), `/v1/models`, `/api/model/options` (models.dev catalog enrichment, `?refresh=true`), `/v1/capabilities`, `/v1/runs` (async runs + SSE events + stop + approval), `/api/sessions` CRUD + chat + chat/stream + `PATCH` (title/end_reason) + `fork` + per-session model lock (enforced on every turn) + `recap`, `/api/jobs` cron HTTP API (CRUD + pause/resume/run), `/v1/skills`, `/v1/toolsets`, `/metrics` (Prometheus counters/gauges — ulnclaw ops extension), `/api/usage` (token accounting: process counters + all-time store totals + per-session rows — ulnclaw ops extension), `/v1/delegations` (background-delegation registry — ulnclaw ops extension), `/v1/browser/status|connect|disconnect` (live CDP endpoint control, hermes `/browser connect` parity — ulnclaw ops extension), bearer-token auth |
| Messaging platforms (Telegram/WhatsApp/QQ/…) | ⬜ deferred | hermes' platform adapters are not ported; the HTTP gateway covers OpenAI-compatible frontends |
| TUI/web/app surfaces | ⬜ deferred | hermes ships a TUI and web apps; ulnclaw is a library + CLI + HTTP gateway today — the `desktop` bridge is the embedding seam a GUI host installs its emitter on |
| Sandbox env scrub + passthrough (`environments/local.py` blocklist, `env_passthrough.py`) | ✅ | terminal/execute_code children get the process env minus a provider/tool credential blocklist and venv markers (`VIRTUAL_ENV`/`CONDA_PREFIX`); skill `required_environment_variables` (registered on `skill_view`) and `[terminal] env_passthrough` allowlist variables — protected provider credentials and `AUXILIARY_*_API_KEY`/`GATEWAY_RELAY_*` dynamic secrets are always refused (hermes GHSA-rhgp-j443-p4rf, fail closed) |
| Environments (`tools/environments/`) | ✅ core | `terminal` backends: local (default), docker (`ensure_docker_container` inspect→run), ssh (BatchMode, identity file); `[terminal] backend/container/image/ssh_host/...`; modal/daytona/vercel deferred |
| Checkpoint manager (`checkpoint_manager.py`) | ✅ | v2 shared shadow git store (`<home>/checkpoints/store`): per-project refs/indexes, transparent pre-edit snapshots (once per turn before `write_file`/`patch`), list/restore/diff/prune CLI, size caps, oversize-file filter, orphan/stale auto-prune |
| Browser supervisor | ✅ | auto-launches managed headless Chrome/Chromium for `ULNCLAW_BROWSER_CDP=auto` |
| Camofox backend (`tools/browser_camofox.py`) | ✅ core | `browser/camofox.rs`: `CAMOFOX_URL` REST anti-detect browser (Camoufox) backend — all 12 browser tools route through REST (tab sessions, accessibility snapshots with refs, click/type/scroll/back/press, image extraction from snapshots, screenshots for vision); CDP overrides take priority; `CAMOFOX_API_KEY` bearer auth, `CAMOFOX_USER_ID`/`CAMOFOX_SESSION_KEY` identity override + existing-tab adoption, Docker loopback URL rewriting (`CAMOFOX_REWRITE_LOOPBACK_URLS` + alias), VNC URL discovery from `/health`, SSRF private-page guard on reads, console/raw-CDP/dialogs report unsupported; managed persistence via `CAMOFOX_MANAGED_PERSISTENCE` (stable UUIDv5 profile-scoped userId, hermes `browser.camofox.managed_persistence`); gateway + REPL browser status report the backend |
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
  context and auto-deny confirm-tier commands by design.  Smart-approval
  (LLM guardian) and cron approval modes are ported; unattended runs fail
  closed unless `cron_mode = "approve"`.
- Browser supervisor launches a local Chrome/Chromium directly; hermes drives
  an external `agent-browser` daemon. The Camofox REST backend is ported
  (incl. managed persistence, via `CAMOFOX_MANAGED_PERSISTENCE` instead of
  hermes' config.yaml knob); other cloud browser providers are not ported.
- The gateway implements the api_server platform subset; profile multiplexing
  (`/p/<profile>/...`) is not ported.  The jobs API delivers locally only
  (`deliver="local"`); hermes' external delivery targets and the NAS/Chronos
  fire webhook (`/api/cron/fire`) are not ported.
- `/api/model/options` enriches the single configured provider row from the
  models.dev catalog (model list + capability/cost maps, `?refresh=true`);
  hermes' multi-provider picker inventory (probing multiple configured
  providers, featured models, credential-pool rows) is not ported.
- Compression uses a char/4 token estimate instead of a tokenizer.
- `patch` fuzzy chain implements all 9 hermes strategies; similarity is an
  LCS-based ratio (difflib.SequenceMatcher stand-in), so edge-case thresholds
  can differ slightly from CPython's matcher.
- Environments cover local/docker/ssh; hermes' modal/daytona/vercel backends
  and their credential flows are not ported.
- Checkpoints skip hermes' legacy pre-v2 store migration (fresh stores only)
  and the volume-identity orphan heuristic (workdir existence is used).
