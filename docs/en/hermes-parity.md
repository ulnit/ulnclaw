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
| `kanban_*` (12 tools) | ✅ full | local SQLite coordination board riding the SAME `KanbanStore` engine and `kanban.db` as the `ulnclaw kanban` CLI and the gateway `/api/kanban/*` endpoints (one board, three surfaces — hermes parity): create (with `parents`)/list/show/comment/heartbeat (auto claim todo→ready→running)/complete/block/unblock/link/attach/attach_url/attachments; unique-prefix id resolution, worker context via `ULNCLAW_KANBAN_TASK`/`HERMES_KANBAN_TASK` (workers default task_id to their own task; create/unblock/link are orchestrator-only, hermes gating), REPL `/kanban` board ops via `run_slash` |
| `browser_*` (12 tools) | ✅ full | CDP WebSocket client (`browser` module): endpoint discovery, page session, accessibility snapshots with element refs, click/type/scroll/press/screenshot/evaluate/dialogs; `ULNCLAW_BROWSER_CDP` accepts ws://, http://host:port, or `auto` (supervisor launches a managed headless Chrome/Chromium); hermes SSRF guards ported (`browser/guard.rs`): sensitive-query + cloud-metadata floor unconditional, private-address guard for non-local endpoints or containerized terminals, post-redirect recheck, JS URL-literal screening for console/eval, raw-CDP allowlist on private pages; browser outputs force-redacted; live endpoint override via REPL `/browser connect` and gateway `/v1/browser/connect|disconnect|status`; Camofox REST backend via `CAMOFOX_URL` (other cloud browser providers remain not ported) |
| `close_terminal`, `read_terminal`, `focus_pane`, `open_preview` | ✅ core | desktop GUI affordances (hermes `close_terminal_tool.py` / `read_terminal_tool.py` / `focus_pane_tool.py` / `open_preview_tool.py`): registered only under `ULNCLAW_DESKTOP=1`, routed through the `desktop` bridge — a host app installs an emitter (`ulnclaw::desktop::set_emitter`) that receives `(ui_session_id, event, payload)` events (`terminal.close`, `pane.reveal`, `preview.open`) plus a blocking `read_terminal` callback; without a wired host they report "desktop only", never kill processes, and normalize bare domains (`www.cnn.com` → https, `localhost:3000` → http); `react_to_message` (hermes `react_to_message_tool.py` port): agent emoji tapbacks — one reaction per author, same-emoji toggles off, defaults to the latest user message (`messages_back` steps earlier, `message_row_id` targets exactly), persisted in `messages.display_metadata` and painted live via the `message.reaction` bridge event; gated on `ULNCLAW_DESKTOP=1` **and** `[display] message_reactions` |
| `computer_use` | ✅ core | cua-driver MCP backend (`src/computer_use.rs`) — full hermes tool schema + approval semantics, see the Computer Use row below; registers once the driver is reachable (`ulnclaw computer-use doctor`) |
| `discord`, `discord_admin`, `feishu_doc_read`, `spotify_*`, `yuanbao` | 🟡 gated | registered, gated on platform credentials; backends pending |
| `x_search` | 🟡 gated | full port of hermes `x_search_tool.py`: xAI Responses-API `x_search` server tool with handle allow/exclude filters (max 10, `@` stripped), strict client-side date-range validation (YYYY-MM-DD, no inverted/pure-future windows), `enable_image_understanding` / `enable_video_understanding`, retry-with-backoff on 5xx/transient failures, `degraded`/`degraded_reason` markers when filters yield no citations, `[x_search]` config (model / reasoning_effort / timeout_seconds / retries); registered only with `XAI_API_KEY` **and** the opt-in `x_search` toolset enabled (hermes parity — SuperGrok OAuth path not ported) |
| `video_analyze` | ✅ core | full port of hermes `vision_tools.video_analyze_tool`: local file / `file://` / HTTP(S) sources (remote downloads gated by the SSRF guard, cached under `cache/video/temp_video_files/` and cleaned up), extension→mime table (mp4/webm/mov/avi/mkv/mpeg/mpg), 20 MB warn + 50 MB base64 hard cap, inline `video_url` data-URL payload, `[auxiliary.vision]` routing with main-provider fallback, one retry on empty responses; opt-in `video` toolset (hermes parity) — requires a provider that accepts video |
| `video_generate`, `bfl_flux3_*` | ✅ core | `video_gen.rs` provider registry (hermes plugin design: single-available auto-select, configured-name fail-closed, `success_response`/`error_response` contract) + unified `video_generate` tool (text/image/reference-to-video, soft validation, model resolution arg > `[video_gen]` config > provider default); `managed_gateway.rs` Nous tool-gateway transport (auth.json bearer + `TOOL_GATEWAY_USER_TOKEN`, `{vendor}-gateway` URL building, presigned `nous-upload:` media uploads); all six `bfl_flux3_*` tools with pinned schemas, local-path upload prep, poll-until-done retrieval (throttle/transport-error handling, 240s backstop), signed-URL download to `~/Downloads` with `.part` staging + collision suffixes + prompting guide; `video_gen_xai.rs` xAI Imagine backend (OAuth access-token reuse from auth.json → `XAI_API_KEY` fallback, text/image-to-video model routing incl. 1.5 model, edit/extend submit+poll flows) + `xai_video_edit`/`xai_video_extend` tools (public-HTTPS-URL validation, `provider_not_configured` gating); `video_gen_backends.rs` FAL backend (six model families — LTX 2.3, Pixverse v6, Veo 3.1, Seedance 2.0, Kling v3 4K, Happy Horse — capability-driven payloads, `FAL_KEY` direct queue REST or Nous `fal-queue` managed gateway) and DeepInfra backend (OpenAI-compatible `/videos` create→poll→download into `~/videos`); no OAuth refresh — cached Nous tokens are used as-is |
| `project_list`, `project_create`, `project_switch` | ✅ core | full port of hermes `tools/project_tools.py` + `hermes_cli/projects_db.py`: per-profile `projects.db` (projects / project_folders / project_meta / discovered_repos, WAL with DELETE fallback + additive column migrations), slug validation + `-2` collision suffixes, multi-folder workspaces with primary pointer (implicit first-folder primary, demote/repoint on removal), archive/restore/hard-delete with folder cascade, active-project pointer, longest-prefix `project_for_path` resolution, deterministic kanban branch names (`<slug>/<task-id>[-<title-slug>]`), repo-discovery cache with policy reconciliation; tools ship in the opt-in `project` toolset (GUI sessions only — off the core coding set, hermes parity) and the host app installs a workspace re-anchor callback (`projects_db::set_project_workspace_callback`) |
| Skill usage telemetry + learning graph (`skill_usage`, `learning_graph`, `learning_mutations`) | ✅ core | hermes `tools/skill_usage.py` + `agent/learning_graph.py` + `agent/learning_mutations.py` ports: `<home>/skills/.usage.json` sidecar (view/use/patch counters, lifecycle state, pinning, agent-created provenance, atomic writes), telemetry wired into `skill_view`/`skill_manage` (bump view/patch, mark agent-created, forget on delete), skill archive/restore via `skills/.archive` (collision timestamp suffixes, pinned skills refused); learning graph payload — learned-skill filter (agent-created or used), `related_skills` edges, memory cards from `MEMORY.md`/`USER.md` bullet entries, lexical memory→skill edges (top-4 per card), clusters + density stats; journey node mutations (`node_detail`/`delete_node`/`edit_node`) aligned with the memory tool's bullet format |
| Learning timeline / `journey` CLI (`learning_graph_render`, `journey`) | ✅ core | hermes `agent/learning_graph_render.py` + `hermes_cli/journey.py` ports: `learning_graph_render.rs` — desktop-ported color math (palette derivation, complementary memory ink, smoothstep age gradient), recency computation (timed + ordinal fallback), day/month/year bucketed timeline with proportional skill/memory bars colored by dominant category (learning heatmap), numbered charted-signal markers, cumulative trajectory sparkline, legend/axis/summary trimmings; `ulnclaw journey` CLI — timeline frame (`--reveal`, `--width/--height`, `--no-color`), `--play` animation, `--json` payload dump, `journey list`, `journey delete <node> [-y]` (skills archived, memories rewritten), `journey edit <node>` via `$EDITOR`; TUI pre-render (`render_frames`) and the GUI star-map remain desktop-only surfaces |
| Skill curator CLI (`curator`) | ✅ core | hermes `hermes_cli/curator.py` local half (the LLM consolidation run stays desktop-side): `curator.rs` — idle-days computation (activity with created_at fallback), prune candidate selection (agent-created, unpinned, non-archived, idle ≥ N days, idlest first), status summary, relative timestamp rendering; `skill_usage.rs` reports — `usage_report` (every skill on disk with provenance/counters/last activity), `unmanaged_report` / `list_unmanaged_skill_names` / `adopt_skill` (provenance stamping), `list_archived_skill_names`; CLI `ulnclaw curator status\|pin\|unpin\|archive\|restore\|list-archived\|usage [--sort activity\|name\|recent] [--json]\|prune [--days N] [--dry-run] [-y]\|adopt [names \| --all-unmanaged] [--dry-run] [-y]\|list-unmanaged`; also hardened the gateway env-override tests with a process-wide env lock |
| Persistent goals / Ralph loop (`goals`) | ✅ core | hermes `hermes_cli/goals.py` port: `goals.rs` — `GoalContract` (outcome/verification/constraints/boundaries/stop_when, alias-table `parse_contract` so an incidental colon isn't mangled, empty-field omission, labelled `render_block`), `GoalState` serde round-trip (status, turn budget, subgoals, parse/transport failure counters, pid/session/time wait barriers), `parse_judge_response` (verdict + legacy `done` bool, code-fence strip, embedded-JSON extraction, wait-directive downgrade when no target), background-process block rendering for the judge; `GoalManager` per-session orchestration persisted in `state_meta` keyed `goal:<session_id>` (set/set_contract/pause/resume/clear/mark_done, subgoal add/remove/clear, wait_on/wait_on_session/wait_for_seconds/stop_waiting with lazy auto-clear, status_line, next_continuation_prompt with contract>subgoals>plain priority, render_contract); fail-open `judge_goal` via the `goal_judge` auxiliary task (contract>subgoals>plain prompt, background processes, transport vs parse failure tracking) + `draft_contract`; `evaluate_after_turn` state machine split into a pure testable `apply_verdict` (wait-barrier short-circuit without burning a turn, WAIT park, DONE, transport auto-pause at 5, parse auto-pause at 3, turn-budget exhaustion, continue) + async judge wrapper; `migrate_goal_to_session`; terminal.rs gains background-process pid capture + `background_process_running`/`background_process_exists`/`list_background_processes` backing session wait barriers; REPL `/goal` (status/show/draft/pause/resume/clear/wait/unwait, inline contract, auto-kick) + `/subgoal` (list/add/remove/clear); `AuxiliaryTaskConfig.max_tokens` config knob |
| Gateway profile multiplexing (`/p/<profile>`) + CDP session liveness | ✅ core | hermes api_server profile-prefix middleware port: every gateway route is mirrored under `/p/<profile>/...`; `[gateway] multiplex_profiles = true` backs each mirror with its own stack (agent from `[profiles.<name>]` override, profile-scoped home `<home>/profiles/<name>` — state.db/approvals.json/cron/skills), lazily built + cached (`ProfileHub`), unknown profile → 404 `Unknown or unconfigured profile`; multiplexing off → prefix accepted but served by the default profile (hermes `_resolve_request_profile` parity); mirrors enforce the same bearer auth. CDP client hardening: `CdpClient.is_connected` (read/write loops flip a closed flag on socket loss and fail in-flight calls fast — no 30s timeout wedge), `with_session` drops dead cached sessions and reopens transparently |
| Startup tips (`tips.py`) | ✅ core | `tips.rs`: feature-discovery one-liner corpus rewritten for ulnclaw's surface (slash commands, goals, CLI subcommands, config knobs, tools, gateway, hidden gems) + dependency-free xorshift64* `get_random_tip`; the chat REPL prints a `✦ Tip:` line at startup and on `/new` (hermes welcome/new-session tip parity) |
| REPL display & composer UX (`hermes_cli/focus_view.py`, `prompt_stash.py`, `clipboard.py`) | ✅ core | `src/focus_view.rs`, `src/prompt_stash.rs`, `src/clipboard.rs` — three hermes CLI-UX modules. **Focus view** (`/focus [on\|off\|status]`): display-only reduced-output mode — snaps tool progress to `off` while remembering the configured mode (restored verbatim on `/focus off`), counts hidden tool lines honestly per turn (only what the configured mode would have shown), prints the `⋯ N tool lines hidden · /focus off to show` recovery line after each turn, plus a `◉ focus` status-bar segment; display-only invariant: never changes what is sent to the model. **Tool progress** (`/verbose [off\|new\|all\|verbose]`): hermes tool_progress_mode cycle over REPL tool-callback scrollback (`⚙ <tool>` lines; `new` dedupes consecutive repeats). **Prompt stash** (`/stash [text\|list\|pop [n]\|drop <n>\|clear]`): session-scoped in-memory stack of parked drafts (the hermes Ctrl+S gesture: content → park, empty + 1 item → pop, empty + 2+ → browse; newest-first, 20-item cap, 60-char previews, `📌 n` prompt indicator, never written to disk). **Clipboard** (`/paste`): cross-platform clipboard-image extraction saved as PNG under `<home>/clipboard/` (macOS pngpaste/osascript, Windows/WSL2 PowerShell WinForms + Get-Clipboard + FileDropList fallbacks, Linux wl-paste on Wayland with non-PNG normalization via ImageMagick and xclip on X11) + `write_clipboard_text` (pbcopy → Set-Clipboard base64 → wl-copy → xclip → xsel, CJK-safe) + SSH-session detection (OSC 52 hint); desktop-side Ctrl+S keybinding stays in the Tauri shell |
| Sessions prune/archive/stats (`session_filters.py`) | ✅ core | `session/filters.rs` — duration parsing (`5h`/`30m`/`2d`/`1w`, bare number = days), point-in-time parsing (durations = that long ago; ISO timestamps naive=local), epoch formatting, `PruneFilters` with typed WHERE-clause builder (ended-only, last-active COALESCE(MAX(message ts), started_at), source/end_reason exact, title/model case-insensitive substring, cwd prefix, message/token/tool-call bounds, tri-state archived) + human-readable `describe()`; store `list_prune_candidates` (oldest-activity-first), `prune_sessions` (messages + FTS first), `archive_sessions` (soft-hide, idempotent), `set_session_archived`, `session_count_by_source`; CLI `ulnclaw sessions prune|archive` (hermes semantics: bare prune = older than 90 days, any filter suppresses the implicit cutoff, bare archive refused, preview + y/N confirm + `--dry-run`, `--include-archived`) and `sessions stats` (totals, per-source counts, db size); hermes' billing/chat/branch/cost filters map to columns ulnclaw doesn't track and stay unported |
| Skin/theme engine (`skin_engine.py`) | ✅ core | `skin.rs`: all 9 hermes built-in skins as data (default, ares, mono, slate, daylight, warm-lightmode, poseidon, sisyphus, charizard — 258 color entries + branding + spinner faces), default-skin inheritance for partial palettes (`build_skin_config`), `list_skins`/`load_skin` (unknown → default), process-wide active skin (`init_skin_from_config` from `[display] skin`, `get/set_active_skin`), `get_color`/`get_branding` accessors, truecolor ANSI `colorize` (NO_COLOR aware); `ulnclaw skins` CLI lists themes with the active marker; REPL tips render in the active skin's `banner_dim`. Deferred: user YAML skins in `<home>/skins/` (no YAML dep), TUI status-bar/prompt-toolkit surfaces |
| Welcome banner & update check (`banner.py`) | ✅ core | `banner.rs`: skin-aware welcome panel rendered with box-drawing chars — braille claw-swipe hero + model line (shortened slug, `.gguf` strip, 28-char cap, models.dev context lookup via `spawn_blocking` + 2s cap), `approvals.mode = "off"` warning (hermes YOLO line), cwd + session id, "Available Tools" grouped by enabled toolsets (8 shown, `+N more toolsets`), skills by category with `+N more` overflow, `N tools · N skills · /help for commands` summary; ULNCLAW block-letter wordmark on terminals ≥95 cols (hermes logo gate); git update check with 6h `$ULNCLAW_HOME/.update_check` cache invalidated on version change — scoped `git fetch` behind-count with shallow-clone SHA-compare path, official-SSH remotes via `git ls-remote` (count unknown → `-1` sentinel), repo dir = `$ULNCLAW_REPO` → build-time `CARGO_MANIFEST_DIR` → `$ULNCLAW_HOME/ulnclaw`; `prefetch_update_check` background thread + `get_update_result(500ms)` while the agent is constructed; panel-title version label `ulnclaw vX · upstream <sha8>` (+carried commits), latest-tag lookup with gitee release URL (per-process cache). Deferred: rich hyperlinks in the title, skin `banner_hero`/`banner_logo` overrides |
| Browser CDP attach layer (`browser_connect.py`) | ✅ core | `browser/connect.rs`: Chromium-family candidate discovery for macOS/Windows/Linux (incl. WSL `/mnt/c` install paths) covering Chrome/Chromium/Brave/Edge; dual-stack loopback CDP probes — `is_browser_debug_ready` (`/json/version` → `/json`, TCP connect for `ws://…/devtools/browser/…`), `discover_local_cdp_url` (IPv4 then `[::1]`, catching browsers pushed to IPv6-only by an IPv4 squatter); port arbitration — `local_port_in_use` distinguishes free-vs-squatted, `find_free_debug_port` requires bindability on both loopbacks; diagnostics-rich visible debug launch `launch_chrome_debug` (per-candidate `LaunchAttempt` states ready/starting/exited/spawn-failed, stderr tail in `<home>/chrome-debug/launch-stderr.log`, exit-0 single-instance absorption hint, `manual_chrome_debug_command` fallback incl. macOS `open -a` form); `connect_local_default` composes the full hermes `/browser connect` default flow. REPL `/browser connect` (no URL) runs that flow, sets the live override on success and injects the hermes system note into the conversation; `/browser disconnect` injects the revert note. Managed-launch candidate list also gains Brave/Edge. Gateway `/v1/browser/*` unchanged (already parity) |
| Doctor (`doctor.py`) | ✅ core | `doctor.rs` + `ulnclaw doctor` CLI: hermes boxed-banner report with ✓/⚠/✗/ℹ checks in sections — Version & Updates (banner git state + 6h-cached upstream behind-count from P61), Configuration Files (config.toml presence/TOML validity/model configured, `.env` key scan), Directory Structure (home + sessions/skills/memory/cron/checkpoints/logs, state.db), Auth Providers (`resolve_api_key` chain: config → ULNCLAW_API_KEY → OPENAI_API_KEY → ANTHROPIC_API_KEY; keyless local providers noted), External Tools (git, Chromium-family candidates from P62, bundled SQLite), Toolsets (enabled/disabled + unknown-name detection via `resolve_toolset`), Skills (installed count + frontmatter sanity), Profiles (per-profile model/toolset overrides + profile home); `--fix` creates missing home/subdirs and a default config.toml (hermes `--fix` fast path), `--online` probes the provider endpoint (`/v1/models` with bearer key; `/api/tags` for ollama-style locals) via blocking reqwest, `--json` emits the serialized report; issues summary with numbered manual steps + `--fix` tip, exit 0 parity (hermes doctor never fails the shell) |
| Session insights (`agent/insights.py`) | ✅ core | `insights.rs` + `ulnclaw insights [--days N] [--source S] [--json]` CLI + REPL `/insights [days]` + gateway chat `/insights [N] [--days N] [--source S]` slash command: InsightsEngine over state.db (second WAL reader) — overview (sessions/messages/tool-calls, in/out/total tokens, avg session duration, active days), models.dev-backed USD cost estimation per session/model (`get_model_info` pricing; provider hinted from config, unknown → "cost unknown"), model breakdown sorted by tokens, source breakdown (hermes platform breakdown), tool-call breakdown from `role='tool'` rows (top 30), activity patterns (hour-of-day + Mon-first weekday buckets, peak detection), top-5 sessions by tokens with title/date; archived sessions excluded, `--source` filter parity, terminal renderer with █ bar charts (hermes `_bar_chart`), `format_duration_compact` + K/M token formatting, JSON report via serde, skill usage breakdown scanning assistant `tool_calls` JSON for `skill_view`/`skill_manage` calls (per-skill loads/edits + last-used dates, summary totals, ranked `top_skills` — hermes `_get_skill_usage`/`_compute_skill_breakdown` semantics), `get_usage_breakdown` tools+skills payload (hermes dashboard-route shape), and the compact markdown `format_gateway` renderer that backs the gateway `/insights` slash reply |
| Pets (`agent/pet/` + `hermes_cli/pets.py`) | ✅ core | `src/pets.rs` + `ulnclaw pets list|install|select|show|off|scale|remove|doctor|hatch`: petdex mascot engine — public manifest fetch (petdex.dev, 300 s in-process cache + background prefetch, petdex-host-pinned asset downloads), profile-scoped store under `<home>/pets/<slug>/` (pet.json + spritesheet: install/load/list/resolve/rename/remove/zip-export/idle-frame thumbnails with anti-traversal slugs), atlas taxonomy inference (8-row legacy vs 9-row Codex sheets) with state aliases (waving/jumping/running), `derive_pet_state` activity→animation mapping (error→failed, celebrate→jump, completed→wave, awaiting-input→waiting, tool-running→run, reasoning→review), and terminal rendering in four modes — kitty graphics protocol (chunked APC transmit + Unicode-placeholder virtual placement payloads with row/column diacritics), iTerm2 inline images, hand-rolled DEC sixel (median-cut ≤255-color quantizer), and a truecolor Unicode half-block fallback with a legibility floor — driven by `[display.pet]` config (enabled/slug/scale 0.1–3.0/render_mode/unicode_cols) persisted by select/off/scale; LLM pet hatch pipeline (`agent/pet/generate/` → `src/pets_atlas.rs` + `src/pets_generate.rs` + `ulnclaw pets hatch`): base-draft → grounded row-strip generation → frame extraction → atlas compose/validation → store registration with hermes-verbatim prompts, chroma-key background removal (border flood-fill + saturated-key fast path + hole repair), xcorr cell registration/normalization, running-left mirroring, idle fallback, 4-wide concurrent row generation with 3 attempts each, `[pets]` config for the OpenAI-compatible images endpoint (image_base_url/image_api_key/image_model; key falls back to OPENAI_API_KEY/ULNCLAW_API_KEY, model to gpt-image-2), `--style` hints (pixel/plush/clay/sticker/flat-vector/3d-toy/painterly/auto), `--drafts N` drafts-only mode and `--base <path>` hatch-from-image; REPL `/pet` (toggle/list/scale/off/<slug> adopt) + `/hatch <description>` slash commands (hermes cli_commands_mixin semantics, progress printing included); P126 ported the desktop generate overlay: the Tauri shell's hatch dialog (prompt + style + draft count → base-draft grid pick → live row progress → spritesheet preview + auto-adopt) rides new gateway hatch jobs (`POST /api/pets/hatch`, `GET /api/pets/hatch/:id`, `POST /api/pets/hatch/:id/pick|cancel`, `GET /api/pets/hatch/:id/draft/:index`). Known diffs: one OpenAI-compatible endpoint instead of hermes' Nous/OpenRouter/Krea provider registry; sheets are PNG-encoded (the `image` crate has no WebP encoder) while decoding accepts both |
| Suggested automations (`cron/suggestions.py` + `suggestions_cmd.py`) | ✅ core | `cron/suggestions.rs`: JSON store at `<home>/cron/suggestions.json` (owner-only writes via tmp+rename) with hermes semantics — pending/accepted/dismissed statuses, dedup-key latching (decided keys never re-offered), MAX_PENDING=5 backlog cap, source validation (catalog/blueprint/usage/integration), resolution by id / 1-based pending index / exact title; `accept` materializes the stored job_spec into a real cron job via `CronStore` and latches accepted; `clear_resolved` prunes accepted records only (dismissed kept for dedup memory); curated 4-entry starter catalog (daily briefing, important-mail monitor, weekly review, workday start reminder — prompts adapted self-contained, schedules verified against `parse_schedule`) with idempotent `seed_catalog_suggestions`; shared dispatch `handle_suggestions_command` behind REPL `/suggestions [accept N|dismiss N|catalog|clear]` and `ulnclaw suggestions` CLI (accept/add/schedule + dismiss/no/reject alias parity, usage text) |
| Status report (`hermes_cli/status.py` + `hermes_cli/subcommands/status.py` + `timefmt.py`) | ✅ core | `status.rs`: port of `show_status` — panel header + Environment (version / home / config.toml / .env), Model+Provider+Base URL, API Keys (config.toml `model.api_key` row + 20-entry vendor env table with alternate fallback, every value passed through `redact_key`), Terminal Backend, Browser (endpoint + binary discovery), Gateway (listen / auth key / multiplex; `--deep` adds gateway-port TCP probe), Scheduled Jobs (active/total + next run), Sessions (total + freshest), Skills (installed + pending suggestions), Updates (git upstream check, 6h cache), footer pointers to doctor/init; `relative_time()` is the timefmt.py port (just now / Nm / Nh / yesterday / Nd / date); CLI `ulnclaw status [--all] [--deep]` (`--all` shares the default redacted rendering) |
| Log viewer + file logging (`hermes_cli/logs.py` + `hermes_cli/subcommands/logs.py` + `hermes_logging.py` rotating handlers) | ✅ core | `logs.rs`: viewer port — `LOG_FILES` registry (agent/errors/gateway), `_parse_since` (Ns/m/h/d cutoffs), timestamp/level/logger-name regexes (logger regex extended for Rust `::` targets), `_matches_filters` (level>= / session substring / since / component prefixes), `_read_last_n_lines` (whole-file <=1MiB, growing backward chunks beyond), `_read_tail` (20x window when filtered), `list_logs` (size + age table), `tail_log` header/filter text parity, `_follow_log` 300ms poll; writer port — `RotatingFile` (max_bytes x backup_count shift rotation, agent.log 5MBx3 INFO+, errors.log 2MBx2 WARNING+, gateway.log 5MBx3 target-filtered) + `HermesLogFormat` (`YYYY-MM-DD HH:MM:SS,mmm LEVEL [session] target: message`) wired into tracing via per-file layers; `COMPONENT_PREFIXES` adapted to ulnclaw module paths; CLI `ulnclaw logs [agent|errors|gateway|list] [-n] [-f] [--level] [--session] [--since] [--component]` |
| Self-updater (`hermes_cli/subcommands/update.py` + `update_cmd.py` git core) | ✅ core | `update.rs`: `--check` port of `_cmd_update_check` — branch resolution (`--branch` > current branch > master, hermes `_resolve_update_branch`), shallow-repo awareness (`--depth 1` fetch + presence-only SHA compare), upstream-preferred fetch for the default branch with origin fallback, fetch-error classification (network / auth / generic), compare-ref verification, behind-count via rev-list; apply path port of `_cmd_update_impl` git core — auto-stash (`--include-untracked`, unmerged-index cleanup, `ulnclaw-update-autostash-<ts>` names), fork detection via origin URL + auto-added `upstream` remote (`_is_fork` / `_add_upstream_remote`, skipped for local-path origins), `git merge --ff-only` (diverged history reported, never force-touched), stash restore with conflict guidance, old..new commit log, then `cargo build --release` as the Rust dependency-refresh equivalent; Python-specific machinery (venv/pip/npm, Windows locking, Tauri/desktop, docker/nix, systemd restarts) is N/A for a compiled Rust binary; CLI `ulnclaw update [--check] [--branch N] [-y]` |
| Backup & restore (`hermes_cli/backup.py` + `hermes_cli/subcommands/backup.py`) | ✅ core | `backup.rs`: full zip backup (hermes `run_backup` — exclusion sets `_EXCLUDED_DIRS/_SUFFIXES/_NAMES` adapted, self-exclusion of the output zip, progress/errors summary, `ulnclaw-backup-<ts>.zip` naming, dir-output handling) with WAL-safe SQLite snapshots via `sqlite backup()` (`safe_copy_db`, hermes `_safe_copy_db`) + `verify_sqlite_integrity`/`is_zeroed_sqlite_file`/`copy_db_and_verify`; import (hermes `run_import` — `validate_backup_zip` markers, `detect_prefix` incl. `.ulnclaw`/`ulnclaw`, zip-slip-guarded staging overlay, `_IMPORT_SKIP_NAMES` runtime-state protection, `_SECRET_FILE_NAMES` 0600 tightening); quick snapshots (hermes `create/list/restore_quick_snapshot` + `_prune_quick_snapshots` — manifest.json, traversal-proof ids, atomic-ish .db replace, keep=20 pruning, max_file_size skip for pre-update); cron safety net `restore_cron_jobs_if_emptied` (counts `cron_jobs` in state.db instead of jobs.json); pre-update hook wired into `ulnclaw update` + pre-import snapshot + post-import safety net in `ulnclaw import`; CLI `ulnclaw backup [-o] [-q] [-l]` / `backup list|restore <id>|prune [keep]` / `ulnclaw import <zip>` |
| Fallback chain CLI (`hermes_cli/fallback_cmd.py` + `fallback_config.py`) | ✅ core | `fallback.rs`: runtime chain already existed (`[model] fallbacks` specs + `agent::with_fallback_specs` / `parse_fallback_spec`); this adds the management CLI — `list` (primary + numbered chain, hermes `cmd_fallback_list` text), `add <provider:model>` (rejects the primary itself via same-deployment compare and exact duplicates, case-insensitive provider), `remove <N|provider:model>`, `clear` (TTY confirm, `-y` to skip); storage written by line-level config.toml editing (`save_chain`: replace/insert `fallbacks = [...]` inside `[model]`, preserving comments/ordering, creates the file when absent); hermes interactive picker replaced by explicit spec argument (ulnclaw has no curses picker); CLI `ulnclaw fallback [list|add|remove|clear] [-y]` |
| Active session leases (`hermes_cli/active_sessions.py`) | ✅ core | `active_sessions.rs`: cross-process lease registry at `<home>/runtime/active_sessions.json` guarded by flock on `active_sessions.lock` (hermes `_FileLock`); entries carry lease_id/session_id/surface/pid + `/proc/<pid>/stat` start time so recycled PIDs cannot spoof liveness (hermes psutil create_time pairing); `prune_dead` reclaims leases of dead processes on every mutation; `try_acquire/release/transfer_active_session`, `release_orphaned_leases`, `active_session_registry_snapshot`, `summarize_holders` ("desktop x4, cli, oldest Nh ago") + `active_session_limit_message` parity; cap configured via `[gateway] max_concurrent_sessions` (0/unset disables; hermes top-level/gateway.* resolution), enforced at chat REPL startup with a Drop-released lease (gateway request path is stateless per-request and not slot-limited) |
| Config management CLI (`hermes_cli/config.py` config_command) | ✅ core | `config_cmd.rs`: `show` (panel header + paths + full config with secret-key redaction via `status::redact_key`), `get <key> [--json]` (dotted paths into config.toml; ALL_CAPS keys resolve through process env + `.env` like hermes `_is_env_config_key`), `set <key> <value> [--force]` (scalar coercion bool/int/float/array/table/string, nested table creation, unknown-section advisory hermes parity; env-style keys written to `.env`), `unset <key>` (config.toml or `.env` line removal), `path` / `env-path`, `edit` ($EDITOR); storage rewrite via toml round-trip (TOML replaces hermes YAML; comments are the documented trade-off) |
| Shell completion (`hermes_cli/completion.py`) | ✅ core | `ulnclaw completion <shell>` via clap_complete: bash / zsh / fish (hermes set) plus elvish / powershell; generated from the live clap command tree, so it tracks subcommands automatically (hermes walks the argparse tree for the same reason); SIGPIPE restored to default so piping into `head` exits cleanly |
| Setup dump & version (`hermes_cli/dump.py`, `build_info.py`) | ✅ core | `ulnclaw dump [--show-keys]`: plain-text, copy-pasteable setup summary — version + git SHA/commit-date, os, profile, home, model/provider, effective terminal backend with `TERMINAL_ENV` override note, `api_keys:` set/not-set/redacted with the shell-only-vs-`.env` mismatch warning (managed backends read `.env`, not the login shell), `features:` toolsets / MCP servers / memory provider / gateway listen+auth / cron active-total / skills / checkpoints, plus non-default `config_overrides:`; `ulnclaw version [--no-update-check]`: version line + install directory/method + live update status via the `update --check` machinery; git-less installs fall back to a baked `.ulnclaw_build_sha` marker (hermes `.hermes_build_sha` parity) |
| Memory CLI (`main.py cmd_memory`) | ✅ core | `ulnclaw memory`: per-store status (entries + bytes for `memory/MEMORY.md` agent notes and `memory/USER.md` user profile, injected into every turn's system prompt); `ulnclaw memory reset [all|memory|user] [--yes]`: hermes-style erase banner (`◆ file (desc) — N bytes`), interactive `yes` confirmation unless `--yes`, per-file `✓ Deleted` report; REPL `/memory` shows current contents |
| Approval mode CLI (`hermes_cli/approval_mode.py`) | ✅ core | `ulnclaw approvals [manual|smart|off]`: shows the effective terminal-approval mode or persists a new one via the canonical config writer (`approvals.mode` in config.toml), re-reads the file to verify the value became effective and reports usage errors / managed-config failures hermes-style; mode semantics (`manual` human prompt, `smart` auxiliary-guardian LLM first, `off` auto-approve outside the hardline floor) match the terminal guard |
| Prompt-size diagnostic (`hermes_cli/prompt_size.py`) | ✅ core | `ulnclaw prompt-size [--json]`: measures the fixed per-call payload — system prompt split into its four tiers (base identity / persistent memory / environment / volatile date+model) with chars + bytes, memory-file sizes, tool count + JSON-schema KB, per-toolset schema sizes largest-first (answers "what should I disable to cut tokens?"), and installed SKILL.md sizes largest-first (skills load on demand, not part of the base prompt); shares `agent::DEFAULT_SYSTEM_PROMPT` and the same building blocks as `Agent::effective_system_prompt`, so the numbers match what is actually injected |
| Debug share bundle (`hermes_cli/debug.py`) | ✅ core | `ulnclaw debug report [--lines N] [--no-redact] [--output DIR]`: collects the hermes-style share bundle locally (no pastebin upload) — `report.txt` (force-redacted `ulnclaw dump` + agent/errors/gateway log tails) plus each present full log, every file self-contained with the dump header and a redaction banner; one snapshot per file derives both tail and full views (rotation-safe), secrets pass through the redaction engine plus email masking, `.1` rotation fallback, on-disk logs never modified |
| Skill bundles (`agent/skill_bundles.py`, `hermes_cli/bundles.py`) | ✅ core | `ulnclaw bundles list|show|create|delete|reload`: YAML bundles in `<home>/skill-bundles/` naming skill sets to load together (`name/description/skills/instruction`, file stem as fallback name, slug normalization shared with skills, duplicate slugs first-wins, broken YAML skipped without breaking discovery); REPL `/<bundle> [instruction]` loads every member skill's SKILL.md into one turn with the hermes header (loaded/missing lists, bundle instruction, user instruction), bundles win over same-named unknown commands, hyphen/underscore interchangeable; missing skills skipped with a note (forgiving `-s` preloading stance) |
| Import agent setups (`hermes_cli/agent_import.py`) | ✅ core | `ulnclaw import-agent [claude-code|codex] [--source DIR] [--dry-run] [--overwrite]`: detect→parse→map→apply with per-item imported/skipped/conflict/error records; claude-code: `CLAUDE.md` → `memory/MEMORY.md` entries (headings become context prefixes, code blocks/tables skipped, dedup), `mcpServers` from `.claude.json` + `settings.json` → config.toml `[[mcp.servers]]` (name conflicts kept, secret-looking env vars stripped and reported), `skills/` → `skills/claude-code-imports/`, permission rules reported as converted patterns (no ulnclaw allowlist surface); codex: `AGENTS.md` + `memories/*.md` → memory entries, `config.toml [mcp_servers.*]` → `[[mcp.servers]]`, `skills/` → `skills/codex-imports/`; memory merges back up the store first (`.bak.<ts>`), 20k-char migration budget; credential files never read, dry-run writes nothing |
| Sessions retitle-skills (`hermes_cli/sessions_cmd.py retitle-skills`) | ✅ core | `ulnclaw sessions retitle-skills [--limit N] [--apply]` (dry run by default): `list_skill_scaffolded_sessions` (titled sessions whose first user turn matches the `[IMPORTANT: The user has invoked the` scaffold), `describe_skill_invocation` re-derives the typed invocation from bundle + single-skill formats (quoted name, `User instruction:` / `alongside the skill invocation:` extraction, excerpt-joint split, whitespace collapse), `generate_title_forced` bypasses the auto-title gate, `_is_titlelike` rejects command-output candidates, unique-title collisions dedupe via `get_next_title_in_lineage` (`base #2`, `#3`, …) |
| Secrets vaults (`agent/secret_sources/`) | ✅ core | `src/secrets.rs` + `ulnclaw secrets status|sync [--apply]`: external secret sources applied at startup before any provider reads env (hermes env-loader hook). Three sources with full hermes precedence semantics — mapped outranks bulk, first claim wins, `preserve_existing` beats everything, `override_existing` beats pre-existing `.env`/shell but never another source, bootstrap-token vars are write-protected. `command`: any KEY=VALUE helper via `/bin/sh -c` (keepassxc-cli / secret-tool / tmpfs cat), hard timeout degrades to "no value", stderr discarded, 1 MiB output cap, quotes/comments parsed. `bitwarden`: Bitwarden Secrets Manager via `bws secret list <project> --output json` (managed `<home>/bin/bws` preferred over PATH, `BWS_SERVER_URL` passthrough, pinned v2.0.0 auto-install from bitwarden/sdk-sm releases — sha256-verified zip, zip-slip-guarded extraction, staged 0755 install). `onepassword`: mapped `op://vault/item/field` bindings resolved via `op read -- <ref>` with a minimal allowlisted child env, empty values refused, per-reference failures degrade to warnings. Fetch errors are one-line warnings, never fatal. TTL fetch caches (`src/secrets_cache.rs` port of `agent/secret_sources/_cache.py`): atomic 0600 writes under `<home>/cache/` (0700 dir), TTL 0 disables both cache layers symmetrically, only complete error-free pulls are cached; the Bitwarden cache is AES-256-GCM **encrypted** at rest (HKDF-SHA256 key derived from the bootstrap token, cache key bound as AAD, legacy plaintext cache deleted on migration). Interactive setup wizards: `secrets bitwarden setup|install|status|token|disable` (hermes 5-step flow — binary install → token → region → project picker via `bws project list` → test fetch → config save; non-TTY fast path requires `--access-token`/`--server-url`/`--project-id`) and `secrets onepassword setup|status|set|remove|disable`. `secrets bitwarden token` rotates the access token without re-running the wizard (hermes `cmd_token`: masked prompt or `--access-token`, `0.` shape warning, probe-before-store via `bws project list` with the NEW credential unless `--no-verify`, configured-project visibility warning, .env persist + both cache layers dropped). Not ported: the Windows bws asset path stays untested |
| Computer Use (`tools/computer_use/`) | ✅ core | `src/computer_use.rs` + `ulnclaw computer-use status|doctor|install`: background desktop control via the cua-driver daemon (MCP over stdio, hermes `cua_backend.py`). Full hermes tool schema (capture som/vision/ax, click family by SOM element index or coordinates, drag, scroll, type, key combos, set_value, wait, list_apps/list_windows/focus_app, cua_browser_* typed-browser passthrough). Hermes precedence/approval semantics: capture + listings are free, every other action goes through the approval callback and fails closed unattended. Lazy shared MCP session (`start_session`/`end_session`, `set_config` max_image_dimension, cursor-overlay policy incl. `--no-overlay` auto-detect + `CUA_DRIVER_RS_TELEMETRY_ENABLED=0` default). `doctor` drives cua-driver's `health_report`. Not ported: macOS TCC `permissions` grant flow, embedded-daemon/socket mode, screenshot eviction + vision post-processing (driver payloads pass through) |
| Plugins (`hermes_cli/plugins.py`, `agent/shell_hooks.py`) | ✅ core | `src/plugins.rs` + `ulnclaw plugins list|enable|disable|accept-hooks`: Rust-native port of the hermes plugin architecture via the shell-hook wire protocol (a static binary can't import Python plugins). Directory plugins at `<home>/plugins/<name>/plugin.toml` (manifest: hooks + `[[tools]]`); tools register as `plugin__<name>__<tool>` and run as subprocesses with `{"tool", "arguments"}` JSON on stdin. Config shell hooks `[hooks] <event> = ["cmd"]` with hermes first-use consent (`shell-hooks-allowlist.json`, `auto_accept` / `ULNCLAW_ACCEPT_HOOKS`). Full hermes `VALID_HOOKS` catalog (23 events); the core fires the 13 hermes emits at runtime: `pre_tool_call` (block decisions veto before approval), `post_tool_call`, `transform_llm_output`, `on_session_start`/`on_session_end`/`on_session_reset` (`/new`)/`on_session_finalize` (REPL exit), `pre_llm_call` (context responses append to the turn's user message, hermes turn-context semantics), `post_llm_call`, `pre_api_request`/`post_api_request`/`api_request_error` (around every provider call), and `pre_gateway_dispatch` (skip/rewrite platform messages BEFORE the allowlist gate); the remaining 10 are catalog-only in hermes v2026.8.3 too. `ulnclaw hooks list|test|revoke|doctor` (hermes `hooks` CLI) inspects consent state, fires default payloads, and probes each consented hook. Hermes Python plugin imports, entry-point packages, and provider registrations are not ported |
| Messaging platforms (`gateway/platforms/`) | ✅ core | `src/messaging.rs` — the hermes platform-gateway architecture runs inside `ulnclaw gateway`: adapters normalize incoming chat messages into a `MessageEvent`, a per-chat session (`platform-<name>-<chat>` via `create_named_session`) carries conversation continuity, and replies go back through the platform with hermes-style chunking. Eleven self-contained adapters — seven long-running loops (Telegram (Bot API long-polling getUpdates/sendMessage), Discord (Gateway v10 websocket IDENTIFY/heartbeat/MESSAGE_CREATE + REST send), Slack (Socket Mode events_api envelopes + chat.postMessage), and Signal (signal-cli HTTP daemon: SSE inbound with keepalive/stale-health reconnect, JSON-RPC 2.0 outbound with rate-limit retry, Note-to-Self promotion + outbound-echo suppression, group gating via `group_allowed_users` (`*` wildcard) with require-mention filtering, attachments via `getAttachment` base64 + mime sniffing + ADTS→m4a ffmpeg remux, `MEDIA:` replies as `base64Attachments`; `[messaging.signal]` or SIGNAL_HTTP_URL/SIGNAL_ACCOUNT), and Weixin (WeChat personal accounts via the Tencent iLink Bot API: long-poll getupdates with persisted sync-buf resume, message-id + content-fingerprint dedup, DM/group intake policies (pairing/allowlist/open/disabled) mapped onto the ulnclaw allowlist∪pairing gate, disk-backed per-peer context_token echo store with session-expired tokenless fallback sends, AES-128-ECB encrypted CDN media both directions (image/video/file/voice with an SSRF host allowlist), 2000-char markdown-aware chunking with copy-friendly line wrapping + text debounce batching, getconfig typing tickets, QR login via `ulnclaw weixin login`; `[messaging.weixin]` or WEIXIN_ACCOUNT_ID/WEIXIN_TOKEN), and QQ (official QQ Bot API v2: WebSocket gateway with Hello/Identify/Resume/heartbeat and hermes close-code semantics (4004 token refresh, 4006/4007/4009 session reset, 4008 rate-limit backoff, 4914/4915 stop), C2C/group @-mention/guild-channel/guild-DM events with 300s message dedup, markdown (msg_type 2) or stripped-plain-text replies riding passive-reply msg_id with `msg_seq` generation, outbound media via inline-base64 under 8 MB or the three-step chunked upload (upload_prepare → presigned COS PUT + upload_part_finish → complete) with daily-quota (40093002) and part-retry (40093001) handling, voice notes via `asr_refer_text` first then raw audio into the `[stt]` pipeline, quoted-message (message_type 103) context merging, INTERACTION_CREATE acknowledgements (inline keyboards and QR scan-to-configure onboarding not ported); `[messaging.qq]` or QQ_APP_ID/QQ_CLIENT_SECRET)`, and Yuanbao (Tencent Yuanbao app bots: WebSocket gateway sessions bootstrapped via an HMAC-SHA256 `sign-token` HTTP handshake (Beijing +08:00 timestamps, cached sign tokens), a hand-rolled protobuf wire codec (`src/yuanbao_proto.rs`: ConnMsg envelopes, AUTH_BIND/BIND_ACK, ping, push acks, 30 s private/group heartbeats), inbound push decoding with a 1.5 s per-sender debounce, DM/group intake policies (pairing/allowlist/open/disabled) mapped onto the allowlist∪pairing gate, markdown-aware 4000-char chunked replies sent over the WS (send-c2c/send-group), hermes no-reconnect close codes (4012/4013/4014/4018/4019/4021), text-only — media/sticker channels not ported; `[messaging.yuanbao]` or YUANBAO_APP_ID/YUANBAO_APP_SECRET/YUANBAO_BOT_ID) plus four gateway-mounted webhook platforms (WhatsApp Cloud, Microsoft Graph change notifications, the generic webhook platform, and BlueBubbles — detailed below). Hermes pairing semantics: every platform is allowlist-gated, empty allowlist fails closed and logs the ids to add. Interactive pairing codes (hermes `gateway/pairing.py` port, `src/pairing.rs`): unauthorized senders receive an 8-char CSPRNG code (salted-SHA-256 at rest, 1-hour expiry, 3 pending per platform, one request per user per 10 minutes, 5 failed approvals lock the platform out for an hour); `ulnclaw pairing list|approve|revoke|clear-pending` manages grants, and approved users join the allowlist as a union at the auth gate (`[messaging] pairing = true` default). Media attachments (hermes media-cache pipeline port, `src/media_cache.rs`): inbound Telegram photo/document/video/audio/voice (getFile download, largest photo size), Discord `attachments`, and Slack `files` (bot-bearer downloads) are cached content-addressed under `<home>/media-cache/` (SHA-256 names, hermes mime→ext table, 25 MB cap) and handed to the agent as path references with vision_analyze/video_analyze/read_file hints (hermes text-fallback semantics); outbound `MEDIA:<path>` reply tags become native uploads on Telegram (sendPhoto/sendDocument), Discord (multipart), and Slack (modern `files.getUploadURLExternal` → PUT → `files.completeUploadExternal` flow); media-only inbound messages flow without text. WhatsApp Cloud (hermes `whatsapp_cloud.py` port, `src/webhook_platforms.rs`): gateway-mounted `/webhooks/whatsapp` with the Meta verify handshake (hub.challenge echo), `X-Hub-Signature-256` HMAC verification over the raw body, text + image/document/audio/video/sticker ingress through the same allowlist∪pairing + plugin gates (inbound media downloaded via the Graph `/media` object with Meta's per-type size caps into the content-addressed cache, captions carried as message text), and Graph-API chunked text replies plus two-step native media sends (`/media` multipart upload → media-id message). Microsoft Graph change-notification ingress (hermes `msgraph_webhook.py` port): `/webhooks/msgraph` validationToken echo + required clientState verification, notifications surfaced as per-resource events (the Teams/Outlook fetcher half is not ported). Generic webhook platform (hermes `webhook.py` port): `[messaging.webhook]` routes mounted at `/webhooks/hook/<name>` with multi-scheme signature verification (Svix `svix-*` headers with base64 `whsec_` keys, GitHub `X-Hub-Signature-256`, GitLab `X-Gitlab-Token`, timestamp-bound generic V2 with a no-V1-downgrade guard, legacy V1, `INSECURE_NO_AUTH` for testing), a 300 s replay window, per-route fixed-window rate limiting (default 30 req/min), delivery-id idempotency (`X-Webhook-Delivery-Id` or `svix-id`, 1 h TTL), header event filtering (`X-Webhook-Event`/`X-GitHub-Event`/`X-Gitlab-Event`), `{event}`/`{body}` prompt templates, delivery targets (`log`/`telegram`/`discord`/`slack`/`whatsapp_cloud`), and `deliver_only` zero-LLM push notifications. BlueBubbles iMessage bridge (hermes `bluebubbles.py` port): `[messaging.bluebubbles]` mounts `/webhooks/bluebubbles` on the gateway with password auth (query param — BlueBubbles webhooks cannot send custom headers — or `x-password`/`x-guid`/`x-bluebubbles-guid` headers), JSON payloads with a form-encoded fallback, hermes event gates (`new-message`/`message`/`updated-message` only; from-me messages and tapback reactions 2000–2005/3000–3005 are silently acked), chat-GUID resolution through an LRU-500 cache with strict `chatIdentifier` matching (no participant fallback — hermes #24157) plus v1.9+ `chats[0]` extraction, attachment downloads into the content-addressed media cache, paragraph-split 4000-char replies with `chat/new` for address targets, multipart attachment sends, and startup ping + server-info + idempotent webhook registration. Inbound voice notes enter the audio STT pipeline (see the Speech-to-text row): transcripts are echoed back as 🎙️ messages and enrich the turn. Interactive clarify prompts render as native WhatsApp buttons/list sheets (see the Interactive clarify row); Telegram/Discord/Slack get numbered-text prompts. Inline keyboards are not ported; Yuanbao is text-only (media/sticker channels remain unported) |
| Interactive clarify (`tools/clarify_gateway.py` + WhatsApp interactive) | ✅ core | `src/clarify_gateway.rs` + messaging integration — the `clarify` tool works in messaging sessions: prompts register in a bounded gateway registry (hermes state cap), render on the platform (WhatsApp `interactive.type=button` ≤3 choices / `type=list` 4+ with the ✏️ Other row, numeric labels with full choice text in the body, 20/24/72-char caps, `cl:<id>:<idx|other>` button ids — hermes `send_clarify` layout), and block the turn until resolved. Taps route through `_dispatch_interactive_reply` semantics: index→choice-text resolution, Other flips to text-capture (`mark_awaiting_text` + ✏️ prompt), unauthorized taps are claimed without dispatch, stale ids fall back to text dispatch with the button title. The next plain message in a session resolves an awaiting clarify instead of starting a turn (hermes `_maybe_intercept_clarify_text`). Non-WhatsApp platforms receive numbered-text prompts; `appr:`/`sc:` prefixes (gateway approvals / slash-confirm) fall through to text like hermes' no-waiter path |
| Speech-to-text (`tools/transcription_tools.py` + gateway STT pipeline) | ✅ core | `src/stt.rs` — hermes' audio STT pipeline: `[stt]` config (enabled/echo_transcripts/provider/language + per-provider blocks with hermes defaults), built-in providers `local_command` (command escape hatch, `ULNCLAW_LOCAL_STT_COMMAND`), `groq` (whisper-large-v3-turbo), `openai` (whisper-1), `mistral` (Voxtral), `xai`, `elevenlabs` (Scribe), `deepinfra` (live-catalog model discovery), all OpenAI-compatible multipart uploads; custom command providers via `[stt.providers.<name>]` + legacy top-level blocks with the built-ins-always-win invariant; gateway voice notes (audio/* attachments) are transcribed before the turn with hermes semantics — local-command fallback on provider failure, empty-transcript sentinel (#41603), neutral failure marker kept out of the prompt, `🎙️ "<transcript>"` echoes (stt.echo_transcripts), WAV/ffprobe duration notes when STT is disabled; `transcribe_audio` agent tool (opt-in `stt` toolset) with model/language overrides. Known difference: hermes' default `local` provider (faster-whisper, Python) cannot embed in the static binary — `stt.local.command` or a cloud provider takes its place |
| OAuth login + skill sync (`hermes_cli/portal_cli.py`, `tools/skills_sync_client.py`) | ✅ core | `src/oauth.rs` + `src/skills_sync.rs`: service-agnostic port of hermes' portal auth + Skill Sync. `ulnclaw auth login` runs the RFC 8628 Device Authorization Grant against any configured `[oauth]` provider (device_authorization_url/token_url/client_id/scopes) with authorization_pending/slow_down handling, token storage at `oauth_tokens.json` (0600), refresh-token grant, `status`/`refresh`/`logout`/`open`. `ulnclaw sync status|pull|push|now|enable|disable|device` keeps hermes' exact UX: opt-in skill sync with stable device id + device label, INERT gate reporting when no `[sync] base_url` is set, pull never clobbers local skills. Transport is generic: HTTP(S) REST with bearer auth (OAuth token or `[sync] api_key`) or a shared directory for offline/NAS sync. Nous-Portal-specific subscription features and org proposal approval flows are not ported |
| Desktop GUI (`apps/desktop` Electron) | ✅ core | `desktop/` — Tauri 2 shell instead of hermes' Electron app (user-requested): Vite/TypeScript webview chat UI (session sidebar with hover rename ✎ / delete 🗑 actions wired to `PATCH`/`DELETE /api/sessions/:id`, SSE token streaming with live tool-progress strip parsing the named `hermes.tool.progress` events into `⚙ <tool> — <status>`, settings, `/`-slash completion popup fed by `/v1/skills` + the gateway command set, expandable tool-call cards (`hermes.tool.started`/`hermes.tool.completed` SSE events with arguments + result panes), clipboard-image paste uploaded via `POST /api/uploads` and attached as hermes text-fallback media path references) talking plain HTTP/SSE to the gateway (`/api/sessions`, `/api/chat` stream, `/api/config`) — no bespoke bridge protocol; the Rust side only manages the `ulnclaw gateway` child (binary lookup PATH → `~/.local/bin`/`~/bin`/`~/.cargo/bin`, port from `[gateway] port`, spawn/SIGTERM). Gateway-side `serve_multiplex` adds a permissive local-app CORS layer (origin echo + OPTIONS preflight; still loopback-bound and key-gated). Browser fallback mode when the Tauri IPC bridge is absent. P119 added a kanban board widget (four-column card wall over `/api/kanban/*`: quick-add, complete/block/unblock actions, comment drawer, board switcher, 5 s polling); P121 added the petdex pet overlay (animated spritesheet canvas over `/api/pets/config` + `/api/pets/:slug/spritesheet`, `display.pet.*` driven, working/idle states from `/v1/runs`, click-to-wave) plus the gateway pet API; P126 added the hatch overlay (hermes pet-generate parity: prompt/style/draft-count form, draft-grid pick, live row progress, spritesheet preview) over a new gateway hatch-job API (`POST /api/pets/hatch` → draft pick → poll → adopt); P163 added the desktop Projects view (third tab over the `/api/projects/*` API: registry cards with create/use/archive/delete, folder management + primary switching, board binding, filesystem scan + discovered-repo adoption); P166 added the Jobs/cron dashboard tab (fourth tab over `/api/jobs`: job rows with status dot, schedule, skills, prompt preview and next-run countdown; pause/resume, run-now, inline edit, delete, create dialog, 10 s polling); the remaining Electron dashboard widgets and tray integrations are not ported |
| Sessions browse (`hermes_cli/sessions_cmd.py browse` + curses picker) | ✅ core | `ulnclaw sessions browse [--source S] [--limit N]`: raw-mode TUI on a TTY (crossterm port of the curses picker — alternate screen, ↑/↓/PgUp/PgDn/Home/End navigation with scrolling, live type-to-filter with backspace, green `▶` selection highlight, Enter selects, bare `q` quits while no filter is active, Esc clears the filter first and quits on the second press, single-step ↑/↓ wrap around the list, a dim column-header strip (Title/Preview · Active · Src · ID) and a bottom footer with the cursor position + filtered-from count, "terminal too small" guard, Enter delivered as LF (`Ctrl+J`) also accepted) with a plain numbered-stdin fallback for pipes/CI; newest-activity-first rows (title → first-user-message preview fallback, relative time, source, truncated id), substring filter over title/preview/id/source, `tool`-source sessions excluded unless `--source` is given (hermes semantics); P165 added the owning-project badge: rows whose cwd falls under a project folder render a `⌂ slug` prefix and the live filter also matches project slugs (both pickers, best-effort against `projects.db`); selection relaunches the current binary with `--resume <id>` (hermes `relaunch`); store query `list_sessions_for_browse` returns the picker rows in one SQL statement |
| Session resume & live-session continuity (`cli.py --resume/--continue`) | ✅ core | Global `-r/--resume <id-or-prefix>` + `-c/--continue` flags on `chat` and `run`: the whole REPL conversation lives in ONE session row (previously every turn created a fresh row); resume seeds the REPL history from `load_messages` (system rows dropped), prints `Resuming session: <id> (title)`, and each turn runs through `run_with_session` against the same id; `/new` rotates to a fresh session key + resets the per-session goal manager; `latest_session_id` picks the `--continue` target by last activity (archived skipped); all id-taking `sessions` actions (`show`/`export`/`recap`/`delete`/`rename`) accept unique prefixes via `resolve_session_id`; hermes' `-c <session-name>` title lookup stays unported (`-c` takes no value) |
| Sessions repair (`hermes_state.py repair_state_db_schema`) | ✅ core | `ulnclaw sessions repair [--check-only] [--no-backup]`: health probe (`db_opens_cleanly` — `PRAGMA journal_mode` first-statement trip, `integrity_check`, sessions read, FTS MATCH read probe, rolled-back FTS write probe) then escalating strategies — FTS5 `'rebuild'` in place, `REINDEX` for stale B-tree indexes, `sqlite_master` de-duplication via `writable_schema` (keeps the FTS index), drop-FTS-schema + `VACUUM` with rebuild on next store open (`initialize_schema` backfills a lagging external-content index); timestamped raw backup + WAL/SHM sidecars first; failure points at offline `sessions recover`; runs before the store opens since a malformed schema is exactly the case where open fails |
| Sessions delete/rename/optimize (`hermes_cli/sessions_cmd.py`) | ✅ core | `ulnclaw sessions delete <id> [--yes]` (id or unique prefix via `resolve_session_id` — LIKE-escaped prefix match, exact id wins, ambiguous → not found; y/N confirm unless `--yes`; messages + FTS rows removed first), `sessions rename <id> <title...>` (hermes `sanitize_title`: ASCII/Unicode control-char stripping, whitespace collapsing, empty → title cleared, 100-char limit, cross-session title uniqueness; reports the stored title), `sessions optimize` (FTS5 `'optimize'` segment merge + best-effort WAL checkpoint + `VACUUM`; reports merged-index count and before/after size using `logical_size_bytes` page accounting so WAL lag can't understate the win) |
| Supply-chain security audit (`hermes_cli/security_audit.py`) | ✅ core | `ulnclaw security audit [--json]`: on-demand OSV.dev audit of pinned MCP server packages (`npx pkg@ver` / `uvx pkg==ver`, scoped npm packages included); unpinned/local entries are skipped silently rather than guessed; `querybatch` + per-vuln detail fetch (severity from `database_specific`/`ecosystem_specific`, deduped fixed versions, summaries truncated at 100 chars); findings sorted by severity and grouped by source, human + JSON rendering; hermes' venv/plugin surfaces don't apply to a static Rust binary |

## Feature parity

| hermes feature | ulnclaw | Notes |
|---|---|---|
| Agent loop with tool calling | ✅ | iteration budget, usage accounting, step callbacks |
| SQLite state store (`hermes_state.py`) | ✅ | sessions/messages/system_prompts/state_meta/async_delegations schema, FTS5 with LIKE fallback, lineage (parent sessions) |
| Session recovery (`session_recovery.py`) | ✅ core | `ulnclaw sessions recover <db> [--out FILE]`: offline, non-destructive — source copied (with WAL/SHM/journal sidecars) to a disposable dir, canonical rows copied into a fresh current-schema db with rowid salvage over damaged tables, orphaned messages get reconstructed session rows, FTS rebuilt, integrity-checked, JSON report; never repairs in place or overwrites the active db |
| Environment probe (`tools/env_probe.py`) | ✅ | one deterministic Python-toolchain line in the system prompt when the terminal backend is local: python3/python versions, pip-module availability, `pip`↔`python3` version mismatch, PEP 668 externally-managed marker (uv neutralizes it); silent on healthy machines; process-wide cache built by a single background worker, callers wait ≤10s then fail open; remote backends (docker/ssh) skip the probe; `[agent] environment_probe` toggle (default true) |
| Context compression (`conversation_compression.py`) | ✅ | budget-triggered, middle-turn summarization via secondary model call, keeps system prompt + first user message + recent tail; summary call honors `[auxiliary.compression]` routing |
| Streaming think-block scrubber (`agent/think_scrubber.py`) | ✅ | `think_scrubber.rs`: stateful suppression of `<think>`/`<thinking>`/`<reasoning>`/`<thought>`/`<REASONING_SCRATCHPAD>` blocks in streamed deltas — every content delta is fed through the state machine in `call_with()` (open tags survive chunk boundaries; unterminated opens are boundary-gated), the held-back partial-tag tail is flushed at stream end, and the non-stream path runs the complete-string `strip_think_blocks`; closed pairs are always suppressed, opens only at block boundaries, so prose that merely mentions a tag is never over-stripped |
| Session title generator (`agent/title_generator.py`) | ✅ | `title_generator.rs`: fire-and-forget auto-titling after the first exchange (background task, never adds latency to the reply) — first-2-user-turns guard, existing-title guard, `[auxiliary.title_generation]` routing (`language` pin, `enabled` kill switch with `is_truthy_value` semantics, default true); 500-char snippets, reasoning-block scrub on the answer, quote/"Title:"-prefix/first-line/80-char cleanup; atomic `set_auto_title_if_empty` persistence so a manual title set while generation was in flight wins; the optional title callback and portal/accounting tags are not ported (no live UI surface) |
| Persistent goals — the Ralph loop (`hermes_cli/goals.py`) | ✅ core | `goals.rs`: standing goals that survive across turns — after every assistant turn a `goal_judge` auxiliary model decides done/continue/wait; continuation prompts are fed back as normal user messages until the goal is achieved, paused/cleared, or the turn budget (default 20) exhausts. Completion contracts (outcome/verification/constraints/boundaries/stop_when) via inline `field: value` goal lines or `/goal draft` (aux-model drafted); subgoals (`/subgoal`) fold into judge + continuation prompts; WAIT verdicts park the loop on a background-process pid/session or a deadline without burning turns (`/goal wait <pid>`, auto-clear on release); fail-open judging with auto-pause after 3 consecutive parse or 5 transport failures; goal state persists in `state_meta` keyed `goal:<session_id>` (survives restarts, `migrate_goal_to_session` for session rotation); REPL `/goal` + `/subgoal` slash commands; kanban goal loop stays desktop-side |
| Timezone-aware clock (`hermes_time.py`) | ✅ | `hermes_time.rs`: IANA timezone resolution `ULNCLAW_TIMEZONE` → `HERMES_TIMEZONE` → config `timezone` → server-local (invalid names warn + fall back, cached with `reset_cache()`); system prompt gets a date-only "Conversation started" line + Model/Provider (byte-stable all day for prefix-cache, hermes PR #20451); compression summaries carry a `Current date` anchor |
| Approval system (`approval.py`) | ✅ | command normalization (backslash-joins, `${IFS}`, comment strip), hardline floor (block), recoverable-costly (confirm); REPL y/N prompt; gateway run approvals (`POST /v1/runs/:id/approval`, once/session/always/deny, SSE `approval.request`), fail-closed `[approvals] timeout` (default 300s), `always` grants persisted across restarts; `[approvals] mode = manual|smart|off` — smart mode asks an auxiliary guardian LLM (prompt-injection-hardened prompt, operator `smart_policy` on the trusted channel) and escalates to a human when unsure, `off` auto-approves below the hardline floor; `cron_mode = deny|approve` governs unattended cron runs (deny = fail-closed default) |
| Threat-pattern scanning (`threat_patterns.py`) | ✅ core | advisory injection scan for tool results re-entering context |
| Toolsets (`toolsets.py`) | ✅ | all 33 toolset definitions incl. composition (`includes`), `coding` default |
| Tool registry (`registry.py`) | ✅ | check_fn gating, toolset grouping, max result size truncation |
| Provider abstraction (`runtime_provider.py`) | ✅ | OpenAI-compatible (OpenAI/OpenRouter/DashScope/Ollama/llama.cpp), native Anthropic Messages transport (`anthropic_messages`: system param, tool_use/tool_result blocks, SSE streaming, max_tokens ceilings, OAuth bearer), keyless local providers |
| Provider fallback chain (`fallback_providers`, `try_activate_fallback`) | ✅ core | `[model] fallbacks = ["provider:model", ...]`: on a failed model call the chain advances (lazy per-entry clients, credential fallback to the main key), the activated fallback stays live for the turn, and the next turn restores the primary (hermes `restore_primary_runtime`); delegated/cron children inherit the specs |
| Auxiliary model routing (`auxiliary_client.py`) | ✅ core | `[auxiliary.<task>]` per-task provider/model/base_url/api_key/key_env overrides (`compression`, `vision`, `title_generation`); `"auto"`/blank inherits the main runtime; main client reused when nothing is overridden |
| models.dev catalog (`agent/models_dev.py`) | ✅ core | `models_dev.rs`: fetches `https://models.dev/api.json` with a three-tier cache — in-memory (1h TTL, stale served immediately while a background thread refreshes) → disk (`$ULNCLAW_HOME/models_dev_cache.json`, any age) → singleflight network with 5-minute process-wide failure backoff; provider ID mapping + identity fallback, context/capability lookups (case-insensitive, `:cloud`/`-cloud` suffix fallback), agentic catalog filters (noise patterns + Google hidden list), `get_provider_info`/`get_model_info`; `ULNCLAW_MODELS_DEV_URL` mirror override (http(s)/file), `ULNCLAW_MODELS_DEV_CACHE` path override; gateway `/api/model/options` enrichment + `?refresh=true`; CLI `ulnclaw models providers\|list\|info\|refresh` |
| Config (`config.yaml`) | ✅ | `config.toml` + `.env` file, profiles, env precedence |
| Skills system | ✅ | discovery, frontmatter, linked files, `/skill-name` invocation scaffolding (hermes `build_skill_invocation_message` port: activation note + skill body + skill-directory/supporting-file hints + user-instruction marker, `skill_usage` bump; recognized by `sessions retitle-skills`) |
| Memory system | ✅ | MEMORY.md/USER.md with prompt injection |
| Cron scheduler | ✅ | job store + schedule parsing + poll loop (`cron::run_scheduler`) |
| MCP client (`mcp_tool.py`) | ✅ core | stdio JSON-RPC: initialize/tools/list/tools/call; `[[mcp.servers]]` config; tools registered as `mcp__<server>__<tool>`; OSV malware preflight for npx/uvx/pipx launches (`osv_check.py` port: MAL-* advisories block, fail-open, 1h verdict cache, `OSV_ENDPOINT`/`OSV_CHECK_CACHE_TTL` overrides) |
| CLI (`hermes_cli/`) | ✅ core | chat REPL with slash commands (incl. `/rollback [N|hash] [file]`, `/rollback diff <N>`, `/diff` checkpoint commands, `/recap`, `/goal` + `/subgoal` standing-goal loop, `/kanban` inline board ops, `/pet` + `/hatch` petdex surfaces), one-shot `run`, sessions/tools/skills/cron/checkpoints subcommands (incl. `sessions export --format md\|html` — SHA256-verified Markdown or standalone HTML + manifest —, `sessions recap`, `sessions recover`, `sessions prune`/`archive`/`stats`/`delete`/`rename`/`optimize`/`repair`/`browse`/`retitle-skills`, `kanban init`/`boards list|create|rm|switch|show|rename|set-workdir`/`create [--project P]`/`list [--workflow-template-id T]`/`show`/`ready`/`assign`/`claim`/`heartbeat`/`done`/`block`/`unblock`/`archive`/`comment`/`link`/`unlink`/`dispatch [--max-spawn N] [--dry-run]`/`gc`/`swarm <goal> --worker ASSIGNEE:TITLE[:skill,skill] [--worker ...] --verifier ASSIGNEE --synthesizer ASSIGNEE [--idempotency-key K] [--json]`/`specify [id | --all]`/`decompose [id | --all]`/`diagnostics [id] [--min-severity S] [--json]`/`schedule`/`promote [--force]`/`reclaim`/`reassign [--reclaim]`/`edit`/`set-model [--provider P]`/`attach`|`attachments`|`attach-rm`/`tail [--follow]`/`stats [--json]`/`watch [--assignee P] [--kinds K] [--interval S]` (kanban task engine: boards, TTL claim locks + stale takeover, hermes status lifecycle with icons, comments + event trail, `kanban_task_*` plugin hooks), `project create/list/show/add-folder/remove-folder/rename/set-primary/use/archive/restore/bind-board/scan [--root P --max-depth N]/repos [--clear]` (first-class project registry anchoring kanban worktrees + git-repo discovery cache), `secrets status/sync/bitwarden setup|install|status|disable/onepassword setup|status|set|remove|disable`, `computer-use status/doctor/install`, `plugins list/enable/disable/accept-hooks`, `hooks list/test/revoke/doctor`, `pairing list/approve/revoke/clear-pending`, `weixin login` (WeChat iLink QR-scan account setup), `auth login/status/refresh/logout`, `sync status/pull/push/now/enable/disable/device`, `uninstall --full/--dry-run/--yes` (code checkout + shell PATH entries + wrapper symlinks + optional home wipe; hermes `uninstall.py` port — Windows registry/env steps not ported)), `moa run/list/delete`, `models providers/list/info/refresh` (models.dev catalog), `skills blueprints/schedule/unschedule`, `diff`, `init` |
| Git working diff (`working_diff.py`) | ✅ | `ulnclaw diff [--staged|--all] [--dir PATH] [paths...]` + REPL `/gitdiff [staged|all]`: working/staged/all modes, untracked files folded in via `git diff --no-index` (50-file cap), timeouts; the checkpoint-based REPL `/diff` remains separate |
| Delegation | ✅ | SubAgentRunner trait, depth limit, child sessions |
| Mixture of Agents (`moa_loop.py`, `moa_config.py`) | ✅ core | `[moa.presets.<name>]` reference fan-out + aggregator synthesis (`ulnclaw moa run/list/delete`, REPL `/moa <prompt>`); parallel references, loud/silent degraded policy, all-failed early return, joined-fallback on aggregator failure; the persistent `provider: moa` facade, traces, and privacy filter are not ported |
| HTTP gateway (`gateway/platforms/api_server.py`) | ✅ core | `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` (session continuity via `X-Ulnclaw-Session-Id`, `stream: true` SSE token streaming with `hermes.tool.progress` events), `/v1/responses` (stateful via `previous_response_id`, `stream: true` Responses-API SSE events), `/v1/models`, `/api/model/options` (models.dev catalog enrichment, `?refresh=true`), `/v1/capabilities`, `/v1/runs` (async runs + SSE events + stop + approval), `/api/sessions` CRUD + chat + chat/stream (slash passthrough: `/help`/`/skills`/`/tools`/`/recap`/`/title`/`/usage` execute without an LLM turn; `/skill-name` + `/<bundle>` invocations expand into hermes skill-scaffold user turns — the hermes gateway/run.py skill-command-sharing port) + `PATCH` (title/end_reason) + `fork` + per-session model lock (enforced on every turn) + `recap`, `/api/jobs` cron HTTP API (CRUD + pause/resume/run), `/v1/skills`, `/v1/toolsets`, `/metrics` (Prometheus counters/gauges — ulnclaw ops extension), `/api/usage` (token accounting: process counters + all-time store totals + per-session rows — ulnclaw ops extension), `/v1/delegations` (background-delegation registry — ulnclaw ops extension), `/v1/browser/status|connect|disconnect` (live CDP endpoint control, hermes `/browser connect` parity — ulnclaw ops extension), `/api/uploads` (binary uploads into the content-addressed media cache — desktop clipboard-image paste), `/api/projects` registry CRUD (folders, primary, archive/restore, active pointer, board binding with the workdir mirror) + `/api/projects/scan|repos` discovery (P162), bearer-token auth. Single-instance guard (P155, hermes `gateway run --replace` contract): the running gateway writes `<home>/gateway.pid` (pid + `/proc` start-time token — PID-reuse proof); a fresh start refuses while another live instance holds the file (stale records self-heal), `--replace` terminates the old instance (SIGTERM → SIGKILL escalation) and takes over, `--force` runs alongside |
| TUI/web/app surfaces | ✅ core | TUI: chat REPL + raw-mode session picker (`sessions browse`); web: HTTP gateway serves OpenAI-compatible + session APIs with local-app CORS, so any browser dashboard works against it; desktop: Tauri shell (`desktop/`, see Desktop GUI row) |
| Sandbox env scrub + passthrough (`environments/local.py` blocklist, `env_passthrough.py`) | ✅ | terminal/execute_code children get the process env minus a provider/tool credential blocklist and venv markers (`VIRTUAL_ENV`/`CONDA_PREFIX`); skill `required_environment_variables` (registered on `skill_view`) and `[terminal] env_passthrough` allowlist variables — protected provider credentials and `AUXILIARY_*_API_KEY`/`GATEWAY_RELAY_*` dynamic secrets are always refused (hermes GHSA-rhgp-j443-p4rf, fail closed) |
| Environments (`tools/environments/`) | ✅ core | `terminal` backends: local (default), docker (`ensure_docker_container` inspect→run), ssh (BatchMode, identity file); `[terminal] backend/container/image/ssh_host/...`; modal/daytona/vercel deferred |
| Checkpoint manager (`checkpoint_manager.py`) | ✅ | v2 shared shadow git store (`<home>/checkpoints/store`): per-project refs/indexes, transparent pre-edit snapshots (once per turn before `write_file`/`patch`), list/restore/diff/prune CLI, size caps, oversize-file filter, orphan/stale auto-prune |
| Browser supervisor | ✅ | auto-launches managed headless Chrome/Chromium for `ULNCLAW_BROWSER_CDP=auto` |
| Camofox backend (`tools/browser_camofox.py`) | ✅ core | `browser/camofox.rs`: `CAMOFOX_URL` REST anti-detect browser (Camoufox) backend — all 12 browser tools route through REST (tab sessions, accessibility snapshots with refs, click/type/scroll/back/press, image extraction from snapshots, screenshots for vision); CDP overrides take priority; `CAMOFOX_API_KEY` bearer auth, `CAMOFOX_USER_ID`/`CAMOFOX_SESSION_KEY` identity override + existing-tab adoption, Docker loopback URL rewriting (`CAMOFOX_REWRITE_LOOPBACK_URLS` + alias), VNC URL discovery from `/health`, SSRF private-page guard on reads, console/raw-CDP/dialogs report unsupported; managed persistence via `CAMOFOX_MANAGED_PERSISTENCE` (stable UUIDv5 profile-scoped userId, hermes `browser.camofox.managed_persistence`); gateway + REPL browser status report the backend |

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
├── media-cache/            content-addressed messaging attachments
├── pairing/                DM pairing stores ({platform}-pending/approved.json)
├── shell-hooks-allowlist.json   hook consent records
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
- The gateway implements the api_server platform subset (profile
  multiplexing under `/p/<profile>/...` IS ported — see the feature table).
  The jobs API delivers locally only (`deliver="local"`); hermes' external
  delivery targets and the NAS/Chronos fire webhook (`/api/cron/fire`) are
  not ported.
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
- Secrets: `secrets sync` dry-runs compare against the env the startup hook
  already applied (hermes behaves the same); the Bitwarden `token` rotation
  subcommand is not ported (edit .env and re-run setup instead), and the
  bws auto-install Windows asset path is untested.
- Computer-use: driver payloads (SOM screenshot b64, AX trees) pass through
  without the hermes PNG post-processing / multimodal eviction layer; the
  macOS TCC `permissions` grant flow and embedded-daemon socket mode are not
  ported; `install` shells out to the upstream trycua installer script.
- Plugins: ulnclaw plugins are subprocesses speaking the hermes shell-hook
  JSON protocol (directory plugins + `[hooks]` config), not Python imports;
  the core fires every hook event hermes v2026.8.3 emits at runtime (13 of
  23 — the other 10 are catalog-only in hermes itself); pre_verify has no
  ulnclaw verify loop to attach to; the `ulnclaw kanban` engine now fires
  the kanban_task_claimed/completed/blocked hooks on claim/done/block, but
  the agent-side kanban_* tools now ride the same KanbanStore engine
  (P119 unified the previously separate tables), and P122 ported the dispatcher tick (`kanban dispatch` CLI +
  `POST /api/kanban/dispatch`: stale-claim reclaim with live-pid extension,
  parent-done todo→ready promotion, ready-task worker spawn via detached
  `ulnclaw run` with ULNCLAW_KANBAN_TASK, live concurrency cap, spawn-failure
  auto-block after 2 tries); P123 added the embedded gateway ticker (`[kanban] dispatch_in_gateway /
  dispatch_interval_secs / max_spawn`, default on/60 s/2) and the hermes
  kanban-stop nudge (one-shot workers that end without kanban_complete/block
  are re-prompted up to 2x, `ULNCLAW_KANBAN_STOP_NUDGE=0` disables); P124 added per-task git-worktree
  isolation (`[kanban] worktrees`, default on: each dispatched worker runs in
  `<repo>/.worktrees/<task-id>` on branch `kanban/<task-id>`, reused across
  respawns; `ulnclaw kanban gc` removes trees of done/archived tasks, branches
  kept); P125 ported the hermes kanban swarm (`hermes_cli/kanban_swarm.py`):
  `ulnclaw kanban swarm <goal> --worker ASSIGNEE:TITLE [--worker ...]
  --verifier ASSIGNEE --synthesizer ASSIGNEE [--json]` builds a
  workers→verifier→synthesizer graph — a root blackboard/audit task
  (created done), N ready workers briefed with the swarm protocol, a
  verifier linked to every worker, and a synthesizer linked to the
  verifier; the topology is posted as a `blackboard` comment + `swarm`
  event, and the existing dispatcher promotes verifier/synthesizer as
  their parents complete (`recompute_ready`); P127 completed the swarm
  surface: worker skills passthrough (`--worker ASSIGNEE:TITLE:skill,skill`,
  verifier pinned to `requesting-code-review`, synthesizer to `humanizer`
  — hermes-verbatim), task-level `skills`/`max_runtime_seconds`/
  `idempotency_key` columns (additive migrations; `kanban create
  --skill X --max-runtime N --idempotency-key K`, same fields on the
  gateway create API), idempotent swarm recovery (same key ⇒ topology
  rebuilt from the root blackboard, no duplicate graph), dispatcher
  `reap_timed_out` (SIGTERM + 5 s grace + SIGKILL, task back to ready
  with a `timed_out` event) and force-loaded skills inlined into the
  spawned worker's founding prompt (hermes passes `--skills` pairs); P128
  ported the triage pipeline (`hermes_cli/kanban_specify.py` +
  `kanban_decompose.py`): `kanban create --triage` parks an idea in a new
  `triage` column, `kanban specify` fleshes it into a Goal/Approach/
  Acceptance-criteria spec via `auxiliary.triage_specifier` and promotes
  triage→todo, `kanban decompose` fans it into a 2-6 child dependency
  graph routed over the profile roster (`[kanban] orchestrator_profile /
  default_assignee / auto_promote_children`; root stays alive as the
  child-of-every-child wake-up card, Kahn cycle-checked, fail-soft
  outcomes for --all sweeps), and `kanban diagnostics` ports the
  `kanban_diagnostics.py` rule engine (hallucinated card ids, phantom
  prose refs, repeated spawn failures, worker crash-loops, stuck-blocked
  > 24 h, block/unblock cycling, stranded-in-ready, triage-without-aux)
  with hermes' thresholds and severity ordering; P129 completed the
  remaining hermes kanban CLI surfaces: schedule/promote (parent-gated,
  --force override)/reclaim/reassign (--reclaim)/edit/set-model, the
  attachments CLI (attach/attachments/attach-rm with stable ids), tail
  --follow event streaming, per-board status stats, and boards
  rename/set-workdir; P130 added the board-wide `kanban watch`
  live event stream (assignee/kind filters, hermes watch backend),
  hermes `board_stats` semantics on `kanban stats` (per-assignee counts
  + oldest-ready age + `--json`) and `kanban dispatch --json`; P131
  ported the gateway notification substrate: the `kanban_notify_subs`
  table (task × platform × chat × thread primary key, caught-up cursor
  snapshot on subscribe, chat_type/profile/metadata self-heal), the
  `kanban notify-subscribe / notify-list / notify-unsubscribe` CLI
  surface, `unseen_events_for_sub` + `advance_notify_cursor` building
  blocks for the gateway notifier, and `kanban log [--tail N]` which
  prints a task's worker log from `<home>/kanban/worker-logs/` with
  hermes' partial-line-safe tail; P132 added the `task_runs`
  attempt-history table (hermes `Run` lifecycle: a run opens on claim
  carrying claim lock/TTL + runtime cap, heartbeats and the spawned
  worker pid are mirrored onto it, and it closes with hermes outcome
  semantics — completed / blocked / reclaimed / timed_out on
  done/block/reclaim/stale-release/timeout, plus instant synthesized
  runs for CLI completes on never-claimed tasks and dispatcher spawn
  failures; re-claim recovers stale active runs as `reclaimed`), the
  `kanban runs [--json] [--state-type status|outcome --state-name V]`
  CLI with hermes' table format, and `latest_run` / `latest_summary`
  store helpers; P133 wired the gateway dispatcher's auto-decompose
  path (hermes `_auto_decompose_tick`): each tick re-reads
  `[kanban] auto_decompose` (default on) /
  `auto_decompose_per_tick` (default 3) live from config so flipping
  the toggle stops a runaway fan-out on the next tick without a
  gateway restart (hermes #49638 fail-safe semantics — config read
  errors disable the pass), then decomposes up to N triage tasks via
  the auxiliary LLM before the dispatch fan-out, logging successes at
  info and no-op skips at debug; P134 completed the remaining hermes
  kanban CLI surface: `kanban context` (full `build_worker_context`
  port — capped body/attachments, prior-attempt run summaries with
  metadata, done-parent handoffs with relative-age staleness hints,
  assignee cross-task role history, capped comment thread; the
  `kanban_show` tool now returns the same `worker_context` so spawned
  workers read it without extra round-trips), `kanban repair`
  (integrity_check + content-addressed quarantine + index-scoped
  REINDEX auto-repair, fail-closed otherwise), `kanban assignees`
  (config roster merged with board assignees, per-status counts),
  `kanban daemon` (hermes-deprecated stub pointing at the gateway,
  `--force` keeps the standalone loop), and `ls`/`new` visible
  aliases; P135 wired notification delivery: the gateway runs a
  kanban notifier loop (hermes kanban_watchers notifier, 5 s tick)
  that polls `kanban_notify_subs`, claims unseen terminal events
  (completed/blocked/gave_up/crashed/timed_out/status, with
  archived/unblocked claimed-but-silent so they can't wedge later
  events), renders hermes' message formats (✔ done + handoff first
  line, ⏸ blocked + reason, ⏱ timed_out, ✖ crashed/gave_up, 🔄
  status, @assignee + [board] tags) and sends them through the
  registered platform sender, advancing the per-sub cursor after
  delivery; subscriptions survive crash/retry cycles and are removed
  only when the task reaches done/archived (cursor handles dedup).
  Scoped vs hermes: no per-profile adapter ownership (single shared
  store), no thread routing or dead-chat drop (PlatformSender exposes
  no failure channel), sends assumed delivered; P136 ported the
  unified failure accounting + circuit breaker (hermes
  `_record_task_failure`): tasks grow `consecutive_failures` /
  `last_failure_error` / `max_retries` columns, every spawn failure
  and timed-out attempt consumes the retry budget, hitting the
  threshold (per-task `max_retries` > dispatcher limit > default 2)
  flips ready→blocked with a `gave_up` event (failures /
  effective_limit / limit_source / trigger_outcome payload), and the
  counter resets on completion and deliberate unblock (hermes
  fresh-start policy). CLI: `kanban create --max-retries N` (>= 1
  validated, matching hermes) and the gateway create API accepts the
  same field; P137 added the dispatcher's worker-health detection
  (hermes `detect_crashed_workers` + `detect_stale_running`): every
  tick immediately reclaims running tasks whose worker pid died
  (30 s launch-grace, `ULNCLAW_KANBAN_CRASH_GRACE_SECONDS` override;
  `crashed` event, run closed with outcome `crashed`, failure counted
  against the breaker) and running tasks past
  `[kanban] stale_timeout_seconds` (hermes
  `dispatch_stale_timeout_seconds`, default 14400, 0 disables,
  re-read live in the gateway loop) whose heartbeat is missing or
  older than an hour (worker SIGTERM→SIGKILL, `stale` event, run
  outcome `stale`, deliberately NOT counted as a failure — hermes
  policy); both surface in `DispatchResult.stale` / `.crashed`;
  P138 hardened the embedded dispatcher (hermes gateway loop): an
  exclusive `flock` singleton lock (`<home>/kanban/dispatcher.lock`)
  guarantees exactly one dispatching gateway per machine — a second
  gateway logs the contention and keeps serving HTTP without
  dispatching — plus stuck-dispatcher telemetry (warn when the ready
  queue stays non-empty for 6 consecutive ticks with zero spawns,
  throttled to 300 s); P139 ported hermes' per-task workspaces:
  tasks gain `workspace_kind` (`scratch` default / `worktree` /
  `dir`), `workspace_path` and `branch_name` columns; `kanban create
  --workspace scratch|worktree|worktree:<path>|dir:<path>` and
  `--branch <name>` (worktree-only, hermes validation text) with the
  gateway create API accepting the same fields; the dispatcher
  resolves the workspace BEFORE spawn (hermes `resolve_workspace` /
  `_resolve_worktree_workspace`): scratch dirs under
  `<home>/kanban/workspaces/<id>`, `dir:` paths must be absolute
  (confused-deputy guard, hermes threat model), worktrees anchor on
  the board `default_workdir` (dispatcher-CWD fallback keeps the
  pre-P139 behaviour; hermes raises instead) and materialize
  `<repo>/.worktrees/<task-id>` on branch `wt/<task-id>` (or
  `--branch`), reusing occupied sibling checkouts via a fresh tree;
  the resolved path + branch are persisted on the task row so retries
  reuse them, resolution errors count as `workspace:` spawn failures
  against the circuit breaker, `kanban claim` resolves + prints the
  workspace (hermes `_cmd_claim`), `[kanban] worktrees=true` keeps
  its meaning for tasks created without `--workspace`, and decompose
  children inherit the root's workspace kind/path (worktree children
  always get their own tree, hermes sibling policy); P140 added the
  respawn guard + duration syntax: `kanban create --max-runtime`
  accepts `30s`/`5m`/`2h`/`1d` as well as bare seconds (hermes
  `_parse_duration`), and the dispatcher defers ready tasks that
  cannot benefit from an immediate retry (hermes
  `check_respawn_guard`) — `rate_limit_cooldown` (latest run ended
  `rate_limited` inside `ULNCLAW_KANBAN_RATE_LIMIT_COOLDOWN_SECONDS`,
  default 300, 0 disables), `blocker_auth` (last failure matches the
  quota/auth pattern), `recent_success` (completed run within 1 h
  without a deliberate re-queue) and `active_pr` (GitHub PR URL in a
  24 h comment window); guarded tasks stay ready, each deferral emits
  a `respawn_guarded` event, and the gateway dispatch API reports
  them; P141 ported wake routing: tasks gain a `session_id` column
  stamped by the agent `kanban_create` tool (and accepted by the
  gateway create API), and when a subscribed task reaches a
  wake-eligible terminal event (`completed` / `gave_up` / `crashed` /
  `timed_out` / `blocked` — hermes `_WAKE_KINDS`) the notifier
  resumes the creator session by self-POSTing the hermes-format wake
  message (`[kanban] Task <id> <status>. …`) to the gateway's own
  `/v1/chat/completions` with `X-Ulnclaw-Session-Id` (hermes
  `_self_post_chat_completion`: loopback for wildcard binds, bearer
  key when configured, 600 s turn ceiling, 2/5/10 s backoff on 429 /
  transient errors, fail-fast on other HTTP errors); the wake runs
  best-effort and detached after the text ping so it cannot stall
  other subscriptions; P142 ported typed block kinds (hermes
  `block_task(kind=…)`): `kanban block --kind dependency` parks the
  task in `todo` (`dependency_wait` event) where parent gating +
  `recompute_ready` promote it automatically once the parents finish
  — no human, no cron; `needs_input` / `capability` / `transient` /
  untyped land in `blocked` with `block_kind` +
  `block_recurrences` persisted, and the unblock-loop breaker routes
  a task to `triage` (`block_loop_detected`) when the same cause
  re-blocks `BLOCK_RECURRENCE_LIMIT` (2) times after unblocks —
  recurrences survive unblock deliberately and reset only on
  completion; `unblock_task` now re-gates on open parents
  (blocked → `todo` while parents remain) matching hermes' invariant
  fix; the agent `kanban_block` tool and the gateway block API accept
  the kind; P143 completed the lifecycle CLI surface: bulk `kanban
  done/block/schedule/unblock/promote/archive` (multiple ids, hermes
  `task_ids` + `--ids`), `kanban done --summary/--metadata` storing
  the structured handoff (full summary + JSON facts) on the closing
  run while the `completed` event carries the first summary line
  (400-char cap) for notifiers, `kanban archive --rm` purging
  already-archived tasks with all related rows (guard: only archived
  tasks delete), `kanban unblock --reason` commenting before
  unblocking, `kanban promote --dry-run/--json` backed by a
  mutation-free `validate_promote`, `kanban watch --tenant`, and
  archive now closes an in-flight run as reclaimed + immediately
  promotes children whose archived parent was the last gate
  (`recompute_ready` treats archived parents as done, hermes
  semantics). P144 added completion recovery: `kanban edit
  --result/--summary/--metadata` rewrites a done task's handoff
  (result text + latest completed run's summary/metadata,
  synthesizing a run row when none exists; emits `edited`), the
  terminal kanban tool gained `summary` + `metadata` so workers hand
  off structured facts, and blocking now writes a `BLOCKED: <reason>`
  comment before the state change (hermes `_cmd_block` parity).
  P145 extended `recompute_ready` to the blocked column: a blocked
  task whose parents are all done/archived auto-recovers to ready
  (preserving `consecutive_failures`, `promoted` event) unless the
  block is sticky — latest `blocked`/`unblocked` event is a
  worker/operator `blocked` (#28712) — or the failure count already
  reached the effective limit (per-task `max_retries` > dispatcher
  `failure_limit` > default 2, #35072); the dispatcher passes its
  configured limit through `dispatch_once`. P146 added the
  non-spawnable gate + health probe: `dispatch_once` takes the
  configured profile set and parks ready tasks whose assignee is not
  a configured profile in `skipped_nonspawnable` (claim-pulled
  control-plane lanes that must never auto-spawn — hermes
  #kanban-dispatcher-crash-loop), and the gateway dispatcher's stuck
  warning now consults `has_spawnable_ready` so a ready queue full of
  lanes reads as "correctly idle", firing only when spawnable work
  (unassigned or known-profile tasks) actually waits. P147 ported
  completion artifacts: `kanban done --artifact <path>` (repeatable),
  the agent `kanban_done` tool (`artifacts` array) and the gateway
  complete API stage files living inside a managed scratch workspace
  into `<home>/kanban/attachments/<task>/` before any cleanup can
  erase them (25 MiB cap, missing/oversized declarations fail the
  completion with rollback), record them as `artifact` attachments
  with `attached` events, merge absolute deliverable paths mentioned
  in summary/result prose, and carry the final paths on the
  `completed` event + run metadata (hermes `kanban_complete(
  artifacts=[...])`, `_persist_scratch_completion_artifacts`,
  `_merge_completion_prose_artifacts`). P148 ported the review
  column: `review` joins the status set (🔍); workers call `kanban
  review <id> [--reason]` / the `kanban_review` tool after opening a
  PR (running → review, worker run closed, `review_requested` event);
  `dispatch_once` grows a review loop sharing the max_spawn cap —
  unassigned review tasks land in `skipped_unassigned`, unknown
  assignees in `skipped_nonspawnable`, claimed review tasks open a
  fresh run without re-gating parents (`claim_review_task`), and the
  `sdlc-review` skill is force-loaded when installed under
  `<home>/skills/`; `has_spawnable_review` joins the gateway health
  probe. P149 added the per-profile concurrency cap: `[kanban]
  max_in_progress_per_profile` (hermes #21582) refuses to spawn for
  an assignee already at its in-flight limit even with global
  headroom — counts seed from the running column each tick and count
  would-be spawns in dry runs; skipped tasks land in
  `skipped_per_profile_capped` (CLI line + dispatch JSON). P150
  ported the anti-hallucination completion gate: `kanban done
  --created-card <id>` (repeatable; also the agent `created_cards`
  array and the gateway complete API) verifies each claimed card —
  it must exist AND be created by the worker's profile, created under
  the worker's task id, or linked as the worker's child. Phantom ids
  emit `completion_blocked_hallucination` and block the completion
  without mutating anything (hermes `HallucinatedCardsError`);
  verified ids ride on the `completed` event, and unresolved
  `t_<hex>` references in summary/result prose are flagged after a
  successful completion via `suspected_hallucinated_references`
  (advisory, hermes `_scan_prose_for_phantom_ids`). P151 added the
  per-tick dispatch lock (#35240): every `dispatch_once` tick runs
  under a non-blocking `flock` on `<kanban.db>.dispatch.lock`; a
  losing dispatcher (e.g. an orphan escaped a service restart)
  returns `skipped_locked = true` with zero DB writes and retries
  next interval — surfaced in the CLI (`dispatch: skipped …`) and the
  dispatch API JSON. P152 added worker log rotation: per-task logs
  under `kanban/worker-logs/` rotate at `[kanban]
  worker_log_rotate_bytes` (default 2 MiB), keep one `.log.1` backup
  generation, and append within a generation so re-spawned attempts
  no longer truncate earlier output (hermes
  `worker_log_rotation_config`). P153 closed the stale-worker race:
  dispatch now claims BEFORE spawning (hermes order) so the run row
  exists at spawn time; workers carry `ULNCLAW_KANBAN_RUN_ID` (hermes
  `HERMES_KANBAN_RUN_ID`) and their completions/blocks pass it as
  `expected_run_id` — an atomic `current_run_id` guard refuses a
  reclaimed attempt instead of clobbering the fresh one (CLI Done/
  Block, the `kanban_complete`/`kanban_block` tools and the gateway
  complete/block APIs all thread it). Spawn/workspace failures of a
  claimed attempt now end the run, release the claim back to ready
  and count the failure (hermes `_record_spawn_failure`). P154 added
  the `[kanban] max_in_progress` global concurrency cap (#33488): a
  tick whose board already runs at/above the cap returns early (the
  backlog stays ready, nothing is bucketed), otherwise the effective
  spawn cap clamps to the tighter of `max_spawn` and
  `max_in_progress` so the running column fills exactly to the cap —
  slow workers (local LLMs, resource-constrained hosts) drain before
  piled-up tasks time out. P154 also ported the one-time
  scratch-workspace tip (hermes `_maybe_emit_scratch_tip`): the first
  scratch workspace materialized across the whole install logs a
  warning that scratch output is ephemeral (deleted when the task
  completes), records a `tip_scratch_workspace` event on the task,
  and touches the `.scratch_tip_shown` sentinel so the tip never
  repeats; worktree/dir workspaces are preserved by design and never
  tip. P156 added kanban goal-mode workers (hermes `create --goal` /
  `--goal-max-turns`): a goal card spawns a worker that wraps its run
  in the Ralph-style judge loop IN THE SAME SESSION — after every turn
  the auxiliary judge (`[auxiliary.goal_judge]`) evaluates the latest
  response against the card's title+body; `continue` feeds a
  continuation prompt, `done` issues one explicit kanban_complete
  nudge and then blocks the card as judged-done-never-finalized, and
  an exhausted turn budget (or a reclaimed/archived task) ends in a
  sticky block for human review. Goal-card completions pass the #38367
  judge gate on the CLI `kanban done` and the `kanban_complete` tool:
  a verdict other than `done` rejects the completion with the judge's
  reason (fail-open when no judge is configured or reachable; the
  gateway `/api/kanban` complete endpoint is deliberately not
  gated). P157 added `create --initial-status running|blocked`
  (hermes `VALID_INITIAL_STATUSES`): `blocked` parks the card for
  human-ops review until unblocked — it wins over `--triage` — while
  `running` keeps the default flow (CLI, gateway create API). P158
  added the workflow-template hooks (hermes `workflow_template_id` /
  `current_step_key` task columns): external workflow engines stamp
  cards at create time (gateway create API carries both fields) and
  query them back via `kanban list --workflow-template-id` (SQL-level
  filter; gateway list API takes the same query param). The template
  engine itself lives outside the board in both projects. P159 wired
  per-task model/provider overrides to the workers (hermes
  `model_override` / `provider_override`): a new `provider` task
  column, `kanban create --provider` / gateway create body,
  `kanban set-model [--provider P]` (provider clears together with
  the model; provider-without-model is rejected — hermes contract),
  global `-m/--model` + `--provider` CLI flags that win over config
  and profile (the flags the spawned `ulnclaw run` worker carries),
  and `dispatch_spawn` now passes `--model` / `--provider` from the
  card. P160 ported hermes' first-class project registry
  (`projects_db` + `project` CLI): a per-profile `projects.db` with
  named multi-folder workspaces (`ulnclaw project
  create/list/show/add-folder/remove-folder/rename/set-primary/use/
  archive/restore/bind-board` — slug uniqueness, primary-folder
  repointing, active-project pointer; `bind-board` also mirrors the
  primary repo into the bound board's `default_workdir`), plus
  `kanban create --project <id|slug>` (CLI + gateway create body):
  the project resolves at create time and anchors the worktree under
  the project's primary repo (`<repo>/.worktrees/<task-id>`) with a
  deterministic `<slug>/<task-id>[-<title-slug>]` branch, stored in
  a new `tasks.project_id` column; unresolvable links drop silently
  (hermes drop-dangling semantics). P161 completed the projects
  subsystem with the repo-discovery scanner hermes never shipped
  (its `discovered_repos` cache table exists but only the Electron
  desktop walks the disk, in TypeScript): `ulnclaw project scan
  [--root PATH ...] [--max-depth N]` finds git checkouts (`.git`
  directory or worktree file; hidden + skip-listed dirs pruned,
  symlinks never followed, nested checkouts included) and records
  them with replace semantics + the `cli-scan:v1` policy key;
  `project repos [--clear]` lists/clears the cache. P162 exposed the
  registry to the gateway for desktop surfaces: `/api/projects` CRUD
  (`PATCH` board binding mirrors the primary repo into the board's
  `default_workdir` exactly like CLI `bind-board`; folders add/remove,
  set-primary, archive/restore, hard delete, active pointer) plus
  `/api/projects/scan|repos` discovery. P164 linked sessions to
  projects: `/api/sessions` rows (list + get) carry a `project` slug
  resolved by longest-prefix cwd match against `projects.db` folders
  (archived projects excluded; a missing store degrades to
  `project: null`), and the desktop sidebar renders it as a badge —
  the hermes desktop session-grouping-by-project contract. Remaining deliberate kanban
  divergences: dispatch-time `default_assignee` application (ulnclaw
  spawns unassigned tasks on the default profile instead of skipping
  them).
- Messaging: media arrives as cached path references the agent inspects
  with vision_analyze/video_analyze/read_file (hermes' native multimodal
  user-turn injection is not ported); voice notes ARE transcribed via the
  `[stt]` pipeline, but the built-in `local` faster-whisper provider needs
  `stt.local.command` or a cloud provider in the static binary; no inline
  keyboards; eleven of hermes' twenty-one platform adapters
  are ported (WhatsApp Cloud + MS-Graph ingress and the generic webhook
  platform ride the gateway webhook routes); the pairing
  flow pairs per sender id like hermes, but the configured allowlist stays
  chat/channel-id based (auth gate = allowlist OR approved pairing).
- OAuth/sync: the flows are provider-agnostic (any RFC 8628 endpoint) rather
  than bound to the Nous Portal; sync moves skill bundles only (no org
  proposal/approval workflow, no subscription gating).

## Completion status

The agent core is at parity with hermes-agent v2026.8.3: every core tool,
the full `sessions` surface (`list/show/search/export/recap/recover/prune/
archive/stats/delete/rename/optimize/repair/browse`), startup resume
(`--resume`/`--continue` with one-session-per-conversation continuity), the
CDP browser client + attach layer + Camofox backend, the HTTP gateway
(including the `/v1/browser/*` live-endpoint control and profile
multiplexing), skills/bundles/memory/goals/checkpoints/cron/insights/doctor/pets,
external secret sources (command helper / Bitwarden / 1Password),
computer-use via cua-driver, the subprocess plugin system,
messaging platform gateways (Telegram/Discord/Slack),
OAuth device-flow login + skill sync,
the Tauri desktop GUI (`desktop/`, replacing hermes' Electron app)
and the rest of the CLI are ported. The `sessions` surface intentionally
omits only: `optimize-storage` (ulnclaw was built on the compact
external-content FTS layout from day one — there is no legacy layout to
migrate) and `-c <session-name>` title lookup (`--continue` takes no
value).

Deliberately not ported (hermes surfaces outside the local-agent scope):
the Electron desktop app (ulnclaw ships the
Tauri `desktop/` shell instead; the Electron-only kanban/dashboard widgets
remain unported),
the plugin/hook/egress system
(Python plugin imports/entry-points and the provider registrations they carry), and small
desktop-UX commands (clipboard, focus_view, prompt_stash, uninstall).
