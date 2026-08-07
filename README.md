# ulnclaw 🦞

[English](#english) | [中文](#中文)

---

## English

**A high-performance AI agent engine written in Rust — a port of [hermes-agent v2026.8.3](https://github.com/NousResearch/hermes-agent/tree/v2026.8.3)**

ulnclaw re-implements the Hermes Agent engine in Rust: the same tool surface
(50+ built-in tools), the same SQLite session/memory/skills/cron storage
layout, the same toolset composition — with native performance and a single
static musl binary. See the [parity matrix](docs/en/hermes-parity.md) for the
full feature-by-feature mapping — core parity with hermes-agent v2026.8.3 is
complete, including the messaging-platform gateways (Telegram/Discord/Slack/Signal/Weixin/QQ/Yuanbao/Email/Mattermost/Matrix/DingTalk/WeCom/Feishu/Home Assistant/SMS (Twilio)/WhatsApp (Baileys bridge)/IRC/ntfy/SimpleX/Teams/LINE/Google Chat/Buzz/Photon (iMessage)/Raft/A2A),
the plugin + shell-hook system, secrets vaults, computer-use, OAuth login +
skill sync + local OAuth upstream proxy, and a Tauri desktop GUI (`desktop/`).

### Key Features

- **🤖 Agent loop** — tool calling with iteration budgets, usage accounting, memory injection, step/tool callbacks
- **🔧 50+ built-in tools** — terminal/process, file read/write/patch/search, web search/extract, X (Twitter) search via xAI (`x_search`, opt-in toolset + `XAI_API_KEY`), video understanding (`video_analyze`, opt-in `video` toolset), memory, todo, session search, clarify, skills, delegation, execute_code, cronjob, vision, image generation, video generation (`video_generate` registry — BFL FLUX 3 via the Nous tool gateway, xAI Imagine incl. edit/extend, FAL six-family queue, DeepInfra), desktop projects (`project_list/create/switch`, opt-in `project` toolset), Discord server tools (`discord`/`discord_admin` — bot token, intents-gated schema, `server_actions` allowlist), Feishu/Lark document tools (`feishu_doc_read` + `feishu_drive_*` comment tools — app credentials via env/secret scope/`[messaging.feishu]`), Spotify tools (7 `spotify_*` tools — playback/devices/queue/search/playlists/albums/library via PKCE OAuth, `ulnclaw spotify-auth login`), Yuanbao tools (5 `yb_*` tools — group info/members, DMs, sticker search/send over the live adapter), cross-channel messaging (`send_message` — send/list/react/unreact across every connected platform via the channel directory + home channels, `MEDIA:<path>` native attachments on Telegram/Discord/Slack, emoji reactions with most-recent-message fallback), learning timeline (`ulnclaw journey`), TTS, Home Assistant, kanban, tool search
- **🧰 Toolsets** — hermes-compatible grouping (`coding`, `web`, `file`, `safe`, `debugging`, ...) with composition and enable/disable policy
- **🛡️ Approval system** — command normalization, hardline floor (auto-block), confirm-before-run for costly operations; REPL prompts plus gateway run approvals over HTTP with fail-closed timeout and persisted `always` grants; tirith pre-exec content scanner (exit-code verdict, auto-install with SHA-256/cosign verification, fail-open circuit breaker)
- **💾 SQLite state** — sessions/messages with FTS5 full-text search, lineage (parent/child sessions), cron jobs, kanban board; offline non-destructive `sessions recover` for damaged databases (rowid salvage, orphan-session reconstruction, FTS rebuild)
- **🗜️ Context compression** — budget-triggered middle-turn summarization via a secondary model call, plus three-layer tool-result persistence: oversized tool outputs (>100K chars, `read_file` exempt) are saved to `ulnclaw-results/` through the terminal backend and swapped for a preview + path, with a 200K-char per-turn aggregate budget spilling the largest results first
- **🤝 Delegation** — parallel sub-agents with isolated contexts and depth limits; top-level delegations run fire-and-forget in the background (live transcripts under `cache/delegation/live/`) and one consolidated result re-enters the conversation when the batch finishes; dispatches and results persist in the SQLite delegation registry, so finished work survives restarts (rows still running after a crash recover as terminal "outcome unknown" reports)
- **🧬 Mixture of Agents** — `[moa]` presets fan a prompt out to reference models in parallel and synthesize their answers via an aggregator (`ulnclaw moa run/list/delete`, REPL `/moa`); set `[model] provider = "moa"` to run the entire agent loop on a preset (persistent facade with per-turn reference caching, `save_traces` JSONL traces, `privacy_filter` PII redaction)
- **🗺️ Model catalog** — models.dev-backed multi-provider inventory with a three-tier cache (memory → disk → network with 5-min failure backoff): `ulnclaw models providers|list|info|refresh`, gateway `/api/model/options` multi-provider picker inventory + `?refresh=true`, `ULNCLAW_MODELS_DEV_URL` mirror override
- **⏰ Cron** — `30m` / `every 2h` / `0 9 * * *` / ISO one-shot schedules with a poll scheduler
- **📐 Blueprints** — skills with a `metadata.hermes.blueprint.schedule` frontmatter become cron jobs (`skills blueprints`, `skills schedule/unschedule`)
- **🛡️ Skills guard** — `skills scan <name>` runs the `skills-guard-v1` static scanner (119 threat patterns, invisible-unicode + structural checks, source trust levels) before you install or run third-party skills; dangerous skills are blocked even from trusted sources
- **🔌 MCP client** — stdio JSON-RPC plus remote Streamable HTTP / SSE transports (`url` + optional `transport = "sse"` + static `headers`; `auth = "oauth"` runs OAuth 2.1 + PKCE with discovery, dynamic registration, loopback callback, token refresh and 401 recovery): any MCP server's tools appear as `mcp__<server>__<tool>`; npx/uvx launches get an OSV malware preflight (MAL-* advisories block, fail-open); `lazy = true` servers register from an on-disk schema cache without spawning — the child starts on first tool call (hermes lazy startup); REPL `/reload-mcp` reconnects servers from fresh config with a prompt-cache invalidation confirm (once/always/cancel); stdio children run with a filtered environment (safe baseline + `XDG_*` + `<home>/.env` keys + declared `env` only — ambient secrets never leak) and `${VAR}`/`${env:VAR}` placeholders interpolate from the secret scope first; the gateway brokers MCP OAuth from the dashboard: `POST /api/mcp/servers/<name>/auth` starts a flow and returns the authorization URL to open, the browser redirect lands on the open `GET /api/mcp/oauth/callback/<server>` route (state-validated), and `GET /api/mcp/oauth/flows/<id>` reports status + discovered tools; 401 recoveries dedupe per failed token, and tokens another process refreshed on disk are picked up on the next request (mtime watch); `/reload-mcp` also works in platform chats — native buttons where the adapter supports them, `/approve`/`/always`/`/cancel` text replies otherwise
- **📡 MCP channel bridge** — `ulnclaw mcp serve [--verbose]` exposes messaging conversations to any MCP client (Claude Code, Cursor, Codex, …) over stdio JSON-RPC: `conversations_list`/`conversation_get`/`messages_read`/`attachments_fetch` across every platform session, `events_poll`/`events_wait` long-poll event stream (200 ms mtime-gated DB polling, no startup replay), `messages_send` via the `send_message` pipeline (incl. standalone Telegram/Discord/Slack REST delivery without a live gateway), `channels_list` targets from the channel directory, and `permissions_list_open`/`permissions_respond` for bridge-session approvals
- **✍️ ACP adapter** — `ulnclaw acp [--verbose]` runs ulnclaw as an Agent Client Protocol stdio server for editors like Zed: session management with history replay, streaming `session/update` notifications (message/thought chunks, tool calls with kind mapping, native plan updates from `todo`), multimodal prompts (image blocks ride the native vision path), cooperative cancel, and tool approvals rendered as `session/request_permission` editor prompts
- **📦 Batch runner** — `ulnclaw batch --dataset-file data.jsonl --run-name my_run [--batch-size N] [--resume]` runs the agent across a JSONL prompt dataset with a parallel worker pool, checkpointed resumption (content-scan + index checkpoint), hermes from/value trajectory files per batch, aggregated tool-usage/reasoning statistics, and a final `summary.json`
- **📨 Send CLI** — `ulnclaw send --to telegram "deploy finished"` (or pipe stdin, `--file`, `--subject`, `--list [platform]`, `--json`, `--quiet`) delivers messages from scripts/cron/CI with no LLM and no running gateway for bot-token platforms — hermes exit-code contract (0/1/2)
- **🌐 Browser automation** — 12 `browser_*` tools over a CDP WebSocket client (accessibility snapshots with element refs, click/type/scroll/press, screenshots + vision, console/eval, raw CDP, dialogs); managed headless Chrome (`ULNCLAW_BROWSER_CDP=auto`), any existing DevTools endpoint, the Camofox anti-detect REST backend (`CAMOFOX_URL`), or on-demand cloud sessions via Browserbase / Browser Use / Firecrawl (`[browser] cloud_provider`), with hermes-grade SSRF guards (metadata floor, private-address gating, redirect rechecks, raw-CDP allowlist) and forced secret redaction on browser output
- **🚪 HTTP gateway** — `ulnclaw gateway`: OpenAI-compatible `/v1/chat/completions` + `/v1/responses` (with session continuity), `stream: true` SSE streaming on both (token deltas, tool-progress/function-call events), async `/v1/runs` with SSE events + approval resolution, sessions API (incl. `PATCH`/fork + enforced per-session model lock), `/api/jobs` cron management (CRUD + pause/resume/run) with a built-in scheduler that auto-runs due jobs, external delivery targets (`deliver`: `origin`/platform/`platform:chat[:thread]`/`all`, resolved against platform senders + home-channel env vars with `[SILENT]` suppression, wrapped headers and failure summaries — `GET /api/jobs/delivery-targets` lists them) and the Chronos NAS fire webhook `POST /api/jobs/fire` (JWT-verified from `[cron.chronos]`, 202 + background run), `/v1/skills` + `/v1/toolsets` discovery, `/api/model/options` (multi-provider picker inventory), Prometheus `/metrics`, token accounting `/api/usage`, background-delegation registry `/v1/delegations`, live browser CDP control `/v1/browser/status|connect|disconnect`, gateway monitoring with content-free OTLP health/diagnostics export (`[monitoring]`, `ulnclaw monitoring status`), bearer auth, and a single-instance pidfile guard (`gateway.pid` with a PID-reuse-proof start token — a second start refuses, `--replace` takes over from the running instance, `--force` runs alongside)
- **🖥️ Terminal environments** — run `terminal` locally (default), in docker (auto container creation), or over ssh (`[terminal] backend`)
- **🩺 Terminal failure intelligence** — failed commands carry actionable guidance: benign exit codes are explained (`grep=1` → "no matches, not an error" as `exit_code_meaning`) and well-known failure shapes get one recovery `hint` (command/module not found, git conflicts, gh field drift & rate limits, permission errors, exits 124/126/137)
- **🔬 Environment probe** — when the terminal backend is local, one deterministic Python-toolchain line (pip/python3 mismatch, missing pip module, PEP 668, missing bare `python`) is injected into the system prompt — silent on healthy machines, background-probed with fail-open timeout (`[agent] environment_probe`)
- **🖥️ Desktop bridge tools** — `close_terminal` / `read_terminal` / `focus_pane` / `open_preview` / `react_to_message` (emoji tapbacks, `[display] message_reactions`) for GUI hosts: gated on `ULNCLAW_DESKTOP=1`, routed through the `desktop` bridge (host-installed emitter receives `terminal.close` / `pane.reveal` / `preview.open` events per UI session); never kill processes, report "desktop only" without a wired host
- **🧹 ANSI stripping** — terminal/execute_code output is cleaned of ECMA-48 escape sequences (colors, cursor moves, OSC titles, 8-bit C1) before it reaches the model, so escapes never leak into context or file writes
- **🔒 Sandbox credential scrub** — terminal/execute_code child processes run with provider & tool credentials stripped from their environment (hermes GHSA-rhgp-j443-p4rf semantics); skills' `required_environment_variables` and `[terminal] env_passthrough` allowlist the rest — provider credentials can never be allowlisted
- **🛡️ Web 工具 SSRF 防护** —— `web_extract` 拦截私有/内网目标（环回、RFC1918、CGNAT、ULA、IPv4 映射 IPv6），并拒绝在 URL 中嵌入凭证；云元数据端点（169.254.169.254 等）永远拦截，重定向逐跳重验；可用 `[security] allow_private_urls` 或 `ULNCLAW_ALLOW_PRIVATE_URLS=true` 放开
- **🛡️ SSRF guard for web tools** — `web_extract` blocks private/internal targets (loopback, RFC1918, CGNAT, ULA, IPv4-mapped IPv6) and refuses URLs embedding credentials; cloud metadata endpoints (169.254.169.254 etc.) are always blocked, redirects are re-validated hop by hop; opt out with `[security] allow_private_urls` or `ULNCLAW_ALLOW_PRIVATE_URLS=true`
- **🚫 Binary guard** — `read_file` refuses ~80 binary extensions (images, archives, executables, fonts, bytecode, databases) with a pointer to vision_analyze/terminal; `.pdf` stays readable
- **📏 Configurable output limits** — `[tool_output] max_bytes/max_lines/max_line_length` tune terminal truncation, read_file pagination, and per-line clamping without patching source
- **🕵️ Secret redaction** — ~55 vendor key prefixes, JWTs, private keys, DB connstrings, auth headers, and env-dump `KEY=value` pairs are masked before output reaches the model; file content gets non-reusable sentinels so truncated keys are never written back
- **📸 Checkpoints** — transparent git-backed snapshots before file edits (shared shadow store, per-project chains), `ulnclaw checkpoints list/restore/diff/prune`
- **📝 Working diff** — `ulnclaw diff [--staged|--all]` shows what changed in a git worktree (untracked files included), REPL `/gitdiff`
- **🌐 Providers** — OpenAI-compatible endpoints (OpenAI, OpenRouter, DashScope, Ollama, llama.cpp) plus a native Anthropic Messages API provider (tool_use/tool_result blocks, SSE streaming, OAuth bearer), keyless local providers; per-task auxiliary routing (`[auxiliary.compression]`, `[auxiliary.vision]`, `[auxiliary.title_generation]`) sends secondary calls to a different provider/model; `[model] fallbacks` failover chain with per-turn primary restore; user-defined `[providers.<slug>]` entries (base_url/api_key/key_env/model/mode) surface in the `/api/model/options` multi-provider picker inventory alongside env-authenticated canonical providers and setup-hint skeletons (`[model_catalog] excluded_providers` hides rows)
- **📡 Messaging platforms** — Telegram/Discord/Slack/Signal adapters run inside `ulnclaw gateway` (`[messaging.*]`, Signal via a signal-cli HTTP daemon, Slack typing status via `assistant.threads.setStatus`, Telegram clarify inline keyboards with callback_query tap routing, Discord clarify buttons with INTERACTION_CREATE routing, Slack Block Kit clarify buttons with block_actions routing), image attachments injected natively into the turn as multimodal content (P226 — ≤ 8 MB `image/*` base64 `data:` URLs on OpenAI-compat/Anthropic providers; `[messaging] multimodal_injection = false` keeps the path-reference flow), WhatsApp Cloud (`[messaging.whatsapp_cloud]` + `ulnclaw whatsapp-cloud` credential wizard with field-shape validation; P271) + Microsoft Graph ingress mount as gateway webhook routes (`/webhooks/whatsapp` HMAC-verified, `/webhooks/msgraph` clientState-verified with receipt dedup, resource filters and prompt templates), plus a generic signed-webhook platform (`[messaging.webhook]` routes at `/webhooks/hook/<name>` — Svix/GitHub/GitLab/HMAC-V2 signature schemes, per-route rate limits, delivery-id idempotency, `deliver_only` zero-LLM push) with CLI-managed dynamic subscriptions hot-reloaded per request at `/webhooks/<name>` (`ulnclaw webhook subscribe|list|remove|test`; 0600-permission secret store; P270), and BlueBubbles iMessage (`[messaging.bluebubbles]` → `/webhooks/bluebubbles` password-authenticated webhook, LRU-cached chat-GUID resolution, REST text/attachment sends), and Weixin personal accounts (`[messaging.weixin]` — Tencent iLink Bot API long-polling, QR login via `ulnclaw weixin login`, AES-128-ECB CDN media both ways, context_token echo sends), and QQ (`[messaging.qq]` — official QQ Bot API v2 WebSocket gateway + REST, markdown replies, chunked media uploads, `asr_refer_text` voice transcripts, inline-keyboard exec approvals with INTERACTION_CREATE routing, QR scan-to-configure onboarding (`ulnclaw qq login`) + full setup wizard with DM-policy and home-channel selection (`ulnclaw qq setup`)), and Yuanbao (`[messaging.yuanbao]` — Tencent Yuanbao app bots over a WebSocket gateway with a hand-rolled protobuf wire codec, HMAC-SHA256 sign-token auth, markdown-aware chunked text replies, outbound image/file media via COS upload, stickers (TIMFaceElem via `STICKER:` reply tags, fuzzy catalog lookup, inbound `[emoji: <name>]` rendering), inbound image/file media resolution + download into the media cache, quoted/observed-media backfill with the `[Replying to: …]` quote pointer, forwarded WeChat chat-record deep parsing (elem_type 1009)), and Email (`[messaging.email]` — IMAP polling + SMTP replies with `Re:` threading, SPF/DKIM/DMARC sender verification), and Mattermost (`[messaging.mattermost]` — REST v4 + WebSocket events, mention gating, thread replies, file upload/download), and Matrix (`[messaging.matrix]` — raw Client-Server API `/sync` loop, mxc media both ways, E2EE not supported), and DingTalk (`[messaging.dingtalk]` — hand-rolled Stream Mode WS, sessionWebhook markdown replies, downloadCode media, 🤔Thinking → 🥳Done emoji reactions, AI streaming cards via card_1_0 when `card_template_id` is set), and WeCom (`[messaging.wecom]` — AI Bot WS gateway, respond-msg markdown, chunked media upload, client-side text-split batching), and Feishu (`[messaging.feishu]` — lark_oapi WebSocket long connection with a hand-rolled protobuf frame codec (hermes default `connection_mode = "websocket"`) or gateway webhook `/webhooks/feishu` with signature-verified events, tenant-token media, Typing/CrossMark processing reactions + inbound reaction routing, interactive approval/update-prompt cards with webhook card-action routing (non-approval clicks become `/card` synthetic commands), Drive document-comment agent + meeting-invite satellites), and Home Assistant (`[messaging.homeassistant]` — WebSocket state_changed stream with closed-by-default watch filters + per-entity cooldowns, replies as persistent notifications, standalone `notify/notify` sender on credentials alone), and SMS/Twilio (`[messaging.sms]` — gateway webhook `/webhooks/twilio` with X-Twilio-Signature HMAC validation, markdown-stripped 1600-char REST replies), and WhatsApp (`[messaging.whatsapp]` — bundled Baileys bridge (`scripts/whatsapp-bridge/`) supervised by the gateway: npm-install hash stamp, pidfile/port stale cleanup, `bridge.log` spawn, two-phase readiness + `scriptHash` staleness handshake; `/health`/`/messages` polling, `/send` + `/send-media` + `/send-poll` + `/send-location` + `/edit` (native polls with poll-based clarify, location pins, message editing), read receipts, self-chat `[owner reply]` intake), and IRC (`[messaging.irc]` — rustls TCP client, NickServ support, channel addressing gate), and ntfy (`[messaging.ntfy]` — topic stream subscription + REST publish, echo-tag loop prevention), and SimpleX (`[messaging.simplex]` — simplex-chat daemon WS client, auto-accept contacts, voice/document media), and Teams (`[messaging.teams]` — gateway webhook `/webhooks/teams` over raw Bot Framework protocol, OAuth2 client-credentials sends, AdaptiveCard exec-approval buttons with default-deny tap gate), and LINE (`[messaging.line]` — gateway webhook `/webhooks/line` with X-Line-Signature verification, reply-token + push fallback, slow-LLM postback buttons, outbound media via token-gated `/line/media` HTTPS serving), and Google Chat (`[messaging.google_chat]` — gateway webhook `/webhooks/googlechat` with Google ID-token verification or Pub/Sub REST-pull inbound, service-account RS256 JWT Chat API sends, per-user OAuth native attachment delivery via in-chat `/setup-files` + `ulnclaw google-chat-oauth`, typing-indicator card patched in-place into the reply), Buzz (`[messaging.buzz]` — Block's Nostr-based platform: NIP-42-authenticated WebSocket subscription (signed kind-22242 auth, hermes `nostr_auth` port) with CLI-poll fallback, kind-9 chat events, require-mention gate, pubkey echo suppression, 👀 seen tapbacks, startup high-water seeding (history never replays) + `dms list`/`channels list` DM discovery, kind-44100 membership-event live DM rediscovery with dynamic `hermes-buzz-dm-<n>` subscriptions, p-tag DM latch classification), Photon (`[messaging.photon]` — iMessage via the Photon Spectrum sidecar HTTP API: `/healthz` gate, typed NDJSON inbound stream, rich-link rendering + preview-art/echo suppression, group wake-word mention gate, `/typing` indicators, URL-only `/send-richlink` replies, allowlist ∪ pairing gate, opt-in `PHOTON_REACTIONS` lifecycle tapbacks + inbound tapback routing), Raft (`[messaging.raft]` — gateway wake endpoint `/webhooks/raft/wake` with bridge-token verified `raft-activity.v1` events + auto-spawned `raft agent bridge` child process), and A2A (`[messaging.a2a]` — Agent2Agent v1.0 server surface: `/.well-known/agent-card.json` discovery + JSON-RPC `POST /a2a` message/send + task ledger): allowlist-gated pairing (fail closed) plus hermes-style interactive pairing codes (`pairing list/approve/revoke/clear-pending`), media attachments cached under `media-cache/` and delivered as path references (outbound `MEDIA:` tags upload natively on Telegram/Discord/Slack; WhatsApp media rides the Graph `/media` endpoint both ways), one persistent session per chat, hermes-style reply chunking. The `clarify` tool works in chats: WhatsApp renders native buttons/list sheets, Telegram inline keyboards, Discord/Slack buttons, other platforms numbered text; button taps and follow-up text resolve the pending question. Slack gets native slash commands: `ulnclaw slack manifest` generates the app manifest registering every platform command as a first-class slash (assistant/agent/flat DM experiences), slash envelopes dispatch through the normal path with `response_url` replies; every platform answers the direct set (`/help` `/skills` `/tools` `/recap` `/title` `/usage` `/insights`) without an LLM turn (P265)
- **🎙️ Voice-note transcription (STT)** — inbound audio/voice messages are transcribed before the agent turn (`[stt]` config): built-in `local_command` / `groq` / `openai` / `mistral` / `xai` / `elevenlabs` / `deepinfra` providers plus custom `[stt.providers.<name>]` command providers, transcripts echoed back as 🎙️ messages and injected into the turn with hermes fallback/sentinel semantics; a `transcribe_audio` tool (opt-in `stt` toolset) covers arbitrary files. The Python-only faster-whisper `local` provider is replaced by `stt.local.command` / cloud backends
- **🐾 Pets (petdex)** — `ulnclaw pets list|install|select|show|off|scale|remove|doctor|hatch`: adopt animated petdex mascots (live gallery of thousands, host-pinned installs under `<home>/pets/`), animate them in the terminal via kitty/iTerm2/sixel graphics or a truecolor half-block fallback, one `[display.pet]` scale knob resizes every surface — or `hatch` a brand-new pet from a text description: LLM base drafts → grounded animation rows → sliced/normalized spritesheet → auto-adopted (OpenAI-compatible images endpoint via `[pets]` config)
- **📋 Kanban engine** — `ulnclaw kanban`: multi-board task engine in `kanban.db` (hermes statuses todo/ready/running/scheduled/blocked/done/archived with icons), TTL claim locks with stale takeover + heartbeats, a stale-run completion guard (workers carry their run id; reclaimed attempts cannot complete/block the fresh attempt), typed blocks (`kanban block --kind dependency|needs_input|capability|transient` — dependency waits in todo until parents finish; same-cause re-block loops escalate to triage), parent→child task links, comments + event trail, board CRUD, and a dispatcher (`kanban dispatch [--max-spawn N] [--dry-run]` + `POST /api/kanban/dispatch`) that reclaims stale claims, crashed workers (dead pid) and heartbeat-stale runners, promotes parent-done tasks (auto-recovering non-sticky blocked tasks whose parents finished, unless they sit at the failure limit), skips ready tasks assigned to unconfigured profiles (`skipped_nonspawnable` claim-pulled lanes), honors `[kanban] max_in_progress_per_profile` (#21582 per-profile in-flight cap) and the `[kanban] max_in_progress` global cap (#33488 — a saturated board skips the tick, otherwise spawns fill to the cap), warns exactly once per install when the first scratch workspace is materialized (`tip_scratch_workspace` event + `.scratch_tip_shown` sentinel), and `create --initial-status blocked` to park a card for human review, workflow-template hooks (`workflow_template_id`/`current_step_key` columns with a `list --workflow-template-id` filter), per-task model/provider worker pinning (`create --model M --provider P`, `set-model [--provider P]` — spawned workers carry global `-m/--provider` flags), and goal-mode workers (`kanban create --goal [--goal-max-turns N]` — the spawned worker loops in its session until the auxiliary judge agrees the card is done or the turn budget blocks it; goal completions pass a judge gate on `kanban done` / `kanban_complete`), runs each tick under a non-blocking `.dispatch.lock` (#35240 — a second dispatcher skips the tick instead of racing), and spawns detached `ulnclaw run` workers — each in a per-task workspace chosen at create time (`kanban create --workspace scratch|worktree|worktree:<path>|dir:<path>` with `--branch`, resolved and persisted by the dispatcher before spawn; `[kanban] worktrees` keeps the legacy worktree default; `kanban gc` cleans finished trees), and a swarm orchestrator (`kanban swarm <goal> --worker ASSIGNEE:TITLE --verifier X --synthesizer Y`: root blackboard task + parallel workers + verifier + synthesizer, promoted by the dispatcher as parents finish), a triage pipeline (`kanban create --triage` parks an idea; `kanban specify` / `kanban decompose` flesh it out or fan it into a routed child graph via the auxiliary LLM — the gateway dispatcher auto-decomposes fresh triage tasks each tick, `[kanban] auto_decompose` re-read live as a safety toggle), a read-only board doctor (`kanban diagnostics`), gateway notification subscriptions (`kanban notify-subscribe / notify-list / notify-unsubscribe` — the gateway notifier loop delivers terminal events (✔ done / ⏸ blocked / ⏱ timed_out …) to the subscribed chats over the messaging platforms, and wakes the creator session — recorded via `session_id` when the agent creates the task — by resuming it with a `[kanban] Task …` turn over the gateway's own chat endpoint), per-task worker logs (`kanban log [--tail N]`, rotated at `[kanban] worker_log_rotate_bytes` — default 2 MiB — with one backup generation), a respawn guard defers ready tasks whose immediate retry cannot help (rate-limit cooldown, quota/auth blocker, recent success, open PR — each deferral logged as a `respawn_guarded` event), per-attempt run history (`kanban runs [--json]`, one `task_runs` row per claim→complete/block/reclaim/timeout attempt), full worker briefs with prior-attempt + parent-handoff context (`kanban context`, also served by the `kanban_show` tool), structured completion handoffs (`kanban done --summary … --metadata '{…}'` on the closing run), completion artifacts (`kanban done --artifact PATH…` — scratch-workspace files staged into `kanban/attachments/<task>/` before cleanup, 25 MiB cap), a review column (`kanban review <id> [--reason]` parks a running task after its PR opens; the dispatcher spawns a review agent — force-loading the `sdlc-review` skill when installed — that claims without re-gating parents), bulk lifecycle commands (`done/block/schedule/unblock/promote/archive` take multiple ids; `archive --rm` purges archived tasks; `promote --dry-run --json`), completion recovery (`kanban edit --result/--summary/--metadata` rewrites a done task's handoff; blocking comments `BLOCKED: <reason>` before the state change), an anti-hallucination gate on completion (`kanban done --created-card ID…` verifies every claimed card before anything mutates; phantom ids block with a `completion_blocked_hallucination` event, unresolved `t_<hex>` prose refs are flagged after completion), DB self-healing (`kanban repair`), `kanban assignees`, and a first-class project registry (`ulnclaw project create/list/show/add-folder/remove-folder/rename/set-primary/use/archive/restore/bind-board` — named multi-folder workspaces in `projects.db` with an active-project pointer; `kanban create --project <id|slug>` anchors the card's worktree under the project's primary repo (`<repo>/.worktrees/<task-id>`) with a deterministic `<slug>/<task-id>[-<title-slug>]` branch, and `project bind-board` mirrors the primary repo as the bound board's `default_workdir`; the registry is exposed to the desktop over the gateway `/api/projects/*` CRUD + `scan`/`repos` discovery endpoints); lifecycle transitions fire the `kanban_task_*` plugin hooks. One board, three surfaces: the agent `kanban_*` tools and the desktop board widget (gateway `/api/kanban/*`) share the same engine and database
- **🔌 Plugins & hooks** — directory plugins (`~/.ulnclaw/plugins/<name>/plugin.toml`: hooks + subprocess tools) and `[hooks]` config shell hooks with hermes first-use consent (`plugins list/install/update/remove/enable/disable/accept-hooks` — install/update/remove manage git-hosted plugins, `hooks list/test/revoke/doctor`); the core fires all 13 hook events hermes emits at runtime (pre/post tool & LLM calls, API request lifecycle, session boundaries, gateway dispatch gating)
- **🔑 Secrets vaults** — external secret sources applied at startup before providers read env (`secrets status/sync`): command helper, Bitwarden Secrets Manager (`bws` — pinned auto-install, AES-GCM-encrypted TTL cache, `secrets bitwarden setup` wizard), 1Password (`op://` refs, `secrets onepassword setup/set`) with full hermes precedence semantics
- **🛡️ Egress firewall** — managed iron-proxy for Docker sandboxes (`egress install/setup/start/stop/restart/reload/status/disable/config`, `/egress` status): sandboxes only ever see minted proxy tokens — real provider keys are swapped in by the daemon on allowlisted hosts and never cross the boundary; pinned v0.39.0 binary with SHA-256 + GPG verification, openssl CA, fail-closed token rules, management-API hot reload (hermes `hermes egress`)
- **🖱️ Computer use** — `computer_use` tool via the cua-driver daemon (MCP over stdio, full hermes schema), approval-gated like hermes; `computer-use status/doctor/install`
- **🔄 OAuth + skill sync** — `auth login` RFC 8628 device flow against any `[oauth]` provider; `sync status/pull/push/now` keeps skills in sync over HTTP(S) or a shared directory; `proxy start` runs a local OpenAI-compatible proxy (`127.0.0.1:8645/v1`) that attaches the stored OAuth bearer (auto-refreshed) to a configured `[proxy] upstream_url` so external apps can ride the subscription
- **🖥️ Desktop GUI** — `desktop/`: a Tauri 2 shell (replacing hermes' Electron app) that hosts the chat UI (session rename/delete hover actions, live tool-progress strip from `hermes.tool.progress` SSE events, `/`-slash completion popup over gateway commands + installed skills, expandable tool-call cards from `hermes.tool.started/completed` SSE events, clipboard-image paste uploaded via `POST /api/uploads` and attached as media path references) a kanban board widget, a Projects view (third tab over `/api/projects/*`: project cards with create/use/archive/delete, folder management with primary switching, board binding, filesystem repo scan + discovered-repo adoption; session rows carry their owning project slug from cwd prefix matches against `projects.db`, rendered as sidebar badges), a Jobs view (fourth tab over `/api/jobs`: scheduled-job rows with status, schedule, skills and next-run countdown; pause/resume, run-now, inline edit, delete and a create dialog, 10 s polling), a Usage view (fifth tab over `/api/usage`: token-accounting dashboard with process/store summary cards — gateway tokens, tool calls, API requests, async runs — plus a per-session table with proportional token bars, and an insights section over `/api/insights` (7/30/90-day windows with top models/tools/sessions, activity peaks and estimated cost), 10 s auto-refresh), a Config view (sixth tab over `/api/config`: flattened config.toml editor with secret redaction, add/remove keys, JSON-scalar parsing and a pending-change badge; also lists .env key names), a Doctor view (seventh tab over `/api/doctor`: runs the `ulnclaw doctor` checks with ✓/⚠/✗/ℹ rows per section, an issues panel, and opt-in online provider probes, plus monitoring and browser-CDP panels over `/api/monitoring` and `/v1/browser/status` showing the OTLP health-export posture, browser configuration state, and a live gateway-log tail over `/api/logs/tail` with level filtering, and the configured MCP servers with transport + OAuth posture over `/api/mcp/servers`, with a one-click OAuth connect for unauthorized servers over `POST /api/mcp/servers/:name/auth` that surfaces the authorization URL and polls the flow to its approved state, plus a kanban diagnostics panel with per-board open/total counts, the current-board status histogram and the blocked-task list), a Webhooks view (eighth tab over `/api/webhooks/subscriptions`: lists dynamic webhook routes with URL copy, signed test-fire and delete, plus a create/update form mirroring `ulnclaw webhook subscribe`), a Runs view (ninth tab over `/v1/runs`: tracked async runs with status badges, result inspection, stop, and live approval resolution — once/session/always/deny — for waiting runs), a Skills view (tenth tab over `/v1/skills` + `/v1/toolsets`: installed-skill catalog with category/description/source path and toolset enabled-state cards with expandable tool lists; tab bar wraps), a Sessions view (eleventh tab over `/api/sessions` + `/api/sessions/:id/messages`: read-only transcript browser with a filterable session list, role headers, expandable tool-call argument chips, Markdown export, a gateway-built recap panel over `/api/sessions/:id/recap` and one-click session forking over `POST /api/sessions/:id/fork`, which also fixed transcript loading for chat resume and the artifacts browser); the Runs view also lists async delegations (`/v1/delegations`) with on-demand consolidated-result expansion, and a background watcher raises sticky toasts plus system notifications when a run starts waiting for approval), a petdex pet overlay (animated spritesheet canvas driven by `display.pet.*`), and a hatch overlay (prompt → base-draft pick → live hatch progress → adopted pet, over the gateway `POST /api/pets/hatch` job API), a system-tray icon (Show/Quit menu, left-click restores the window), a per-session model picker (the chat-header model badge opens an overlay over `/api/model/options` and locks the session via `POST /api/sessions/:id/model`), find-in-chat (Ctrl/Cmd+F floating bar with match highlighting and stepping), a command palette (Ctrl/Cmd+K fuzzy launcher for views, session switching, and session actions), an artifacts browser (transcript scan for links/files/images with kind filters), a learning view (learned skills + memory graph with node edit/archive over `/api/learning/*`), dual notification stacks (top-center sticky error/action toasts + bottom-right ambient confirmations) with a cold-boot CONNECTING overlay (P249), a first-run onboarding wizard (welcome → provider setup guidance over `/api/model/options` → finish, latched in localStorage, replayable from the settings menu; P250), full UI i18n in five languages (en/zh/zh-hant/ja/ar) with a searchable globe language switcher and RTL layout for Arabic (P251), a model-visibility editor (curated per-provider model sets with `-fast` family collapsing, hide-all semantics and featured/top-N defaults) filtering the model picker (P252), a boot-failure recovery overlay (Retry / Open settings / Dismiss) that replaces the cold-boot failure toast (P253), session export (hover ⭳ on a session row or command-palette actions download the transcript as Markdown or standalone HTML via `GET /api/sessions/:id/export`), a session picker (type-to-filter `/resume`/`/sessions`/`/switch` overlay intercepted in the composer; P254), chat intro copy on empty sessions (seeded headline/body pairs; P255), a streaming activity timer (elapsed seconds in the tool-progress strip plus measured per-tool durations; P256), the hermes hotkey subset (Ctrl/Cmd+Shift+M model picker, Ctrl/Cmd+N / Shift+N new session, Ctrl+Tab session cycling, Ctrl/Cmd+Shift+F session picker, Ctrl/Cmd+, settings, Ctrl/Cmd+B sidebar toggle, type-to-focus composer; P258), and manages the gateway child process — spawning it with `ULNCLAW_DESKTOP=1` so the desktop affordance tools (`close_terminal`/`read_terminal`/`focus_pane`/`open_preview`/`react_to_message`) reach the webview over the `/api/desktop/events` SSE bridge (with a `terminal.read` HTTP round-trip); the gateway's local-app CORS also serves any browser dashboard

### CLI Quick Start

```bash
cargo build --release --target x86_64-unknown-linux-musl

# Interactive onboarding wizard (provider, terminal, platforms, tools)
./ulnclaw setup                 # or: ./ulnclaw setup model|terminal|gateway|tools|agent
./ulnclaw model                 # switch provider/model interactively
./ulnclaw gui                   # launch the Tauri desktop app (alias: desktop)

# Write a default config to ~/.ulnclaw/config.toml
./ulnclaw init

# One-shot run
./ulnclaw run "Summarize the README.md file"

# Interactive chat (slash commands: /new /search /memory /skills /sessions /rollback /diff /recap /goal /subgoal /focus /verbose /stash /kanban /pet /hatch /paste ...)
./ulnclaw chat
./ulnclaw chat --resume <session-id>   # resume a session by id or unique prefix (-r)
./ulnclaw chat --continue              # continue the most recent session (-c)
./ulnclaw chat --continue "my task"    # ...or the session matching a title/id

# Management subcommands
./ulnclaw tools            # list toolsets and enabled tools
./ulnclaw sessions list    # recent sessions from state.db
./ulnclaw sessions search "auth refactor"
./ulnclaw sessions export <session-id> --out ./exports --format md|html
./ulnclaw sessions recover ./damaged-state.db   # offline db recovery
./ulnclaw sessions repair          # repair malformed state.db schema (--check-only)
./ulnclaw sessions browse          # interactive picker: filter + resume sessions (⌂ project badge, right-hand details pane, Tab source filter, F2 sort toggle)
./ulnclaw sessions retitle-skills  # fix titles that leaked /skill scaffolds (--apply)
./ulnclaw sessions delete|rename|optimize # per-session delete / rename / FTS-merge + VACUUM
./ulnclaw skills list
./ulnclaw skills blueprints    # schedulable skills (skills schedule <name>)
./ulnclaw skills scan <name>   # security scan before trusting a skill (--json, --source, --force)
./ulnclaw cron list          # cron jobs (cron run <id> executes one immediately)
./ulnclaw suggestions        # suggested automations (accept/dismiss/catalog/clear)
./ulnclaw moa list           # MoA presets (run: ./ulnclaw moa run "<prompt>")
./ulnclaw models providers   # models.dev catalog (list/info/refresh)
./ulnclaw checkpoints list   # filesystem snapshots ([checkpoints] enabled = true)
./ulnclaw diff               # git working-tree diff (--staged / --all)
./ulnclaw doctor             # diagnose config/deps (--fix, --online, --json)
./ulnclaw insights           # usage analytics over sessions (--days, --source, --json)
./ulnclaw pets               # petdex mascots: list/install/select/show/scale/doctor/hatch
./ulnclaw status             # status of all components (--deep)
./ulnclaw logs               # tail/filter logs (-f, -n, --level, --since, --component)
./ulnclaw update --check   # check for updates (ulnclaw update applies: stash -> ff pull -> rebuild)
./ulnclaw config           # show/get/set/unset config (env-style keys go to .env)
./ulnclaw secrets status   # external secret sources (secrets sync [--apply] fetches now)
./ulnclaw secrets bitwarden setup   # wizard: install bws, store token, pick project (also: install/status/token/disable; onepassword setup/status/set/remove/disable)
./ulnclaw computer-use status # background desktop control via cua-driver (doctor/install)
./ulnclaw plugins list      # plugins + shell hooks (enable/disable/accept-hooks)
./ulnclaw kanban list       # kanban task engine (init/boards/create [--max-runtime 30s|5m|2h|1d --max-retries N --workspace KIND --branch B --goal --initial-status blocked --model M --provider P --project P]/claim/done/review/block/comment/swarm/specify/decompose/diagnostics/schedule/promote/reclaim/reassign/edit/set-model [--provider P]/attach/tail/log/runs/context/repair/assignees/notify-subscribe/stats/...)
./ulnclaw project list     # first-class project registry (create/show/add-folder/remove-folder/rename/set-primary/use/archive/restore/bind-board; kanban create --project anchors the worktree)
./ulnclaw project scan     # git-repo discovery into the cache (scan [--root P --max-depth N]/repos [--clear])
./ulnclaw hooks doctor      # probe every consented hook (list/test/revoke)
./ulnclaw pairing list      # DM pairing codes for unknown senders (approve/revoke/clear-pending)
./ulnclaw weixin login      # WeChat iLink QR-scan login for [messaging.weixin]
./ulnclaw auth login        # OAuth device-flow login (status/refresh/logout/open)
./ulnclaw sync status       # skill sync (pull/push/now/enable/disable/device)
./ulnclaw completion bash  # shell completions (bash/zsh/fish/elvish/powershell)
./ulnclaw dump             # copy-pasteable setup summary for support (--show-keys)
./ulnclaw version          # version + install info + update status
./ulnclaw uninstall        # remove code/PATH entries/wrappers (--full wipes data, --dry-run, --yes)
./ulnclaw memory           # persistent memory status; `memory reset [all|memory|user]`
./ulnclaw approvals        # terminal approval mode; `approvals manual|smart|off` to set
./ulnclaw prompt-size      # system prompt + tool-schema footprint (--json)
./ulnclaw debug report     # redacted diagnostic bundle for support (--no-redact)
./ulnclaw bundles          # skill bundles: load N skills under one /command
./ulnclaw import-agent     # import Claude Code / Codex setups (--dry-run)
./ulnclaw security audit   # OSV.dev audit of pinned MCP packages (--json)
./ulnclaw fallback         # fallback chain (add/remove/clear provider:model entries)
./ulnclaw backup           # zip backup of home (-q quick snapshot, backup list/restore/prune)
./ulnclaw import b.zip     # restore a backup zip (runtime-state files skipped, secrets 0600)

# Browser automation: auto mode launches a managed headless Chrome/Chromium;
# or point browser_* tools at an existing browser with remote debugging
export ULNCLAW_BROWSER_CDP=http://127.0.0.1:9222     # or ws://.../devtools/browser/... or "auto"
# or route browser_* through a Camofox anti-detect browser server:
# export CAMOFOX_URL=http://127.0.0.1:9377            # + optional CAMOFOX_API_KEY
# or let browser_* spin up on-demand cloud sessions (hermes cloud providers):
#   config.toml [browser] cloud_provider = "browserbase" | "browser-use" | "firecrawl" | "local"
#   + credentials: BROWSERBASE_API_KEY+BROWSERBASE_PROJECT_ID / BROWSER_USE_API_KEY / FIRECRAWL_API_KEY

# HTTP gateway (OpenAI-compatible API server, default 127.0.0.1:8642)
./ulnclaw gateway --host 127.0.0.1 --port 8642
# messaging platforms run inside the gateway
# ([messaging.telegram|discord|slack|signal|weixin|qq|yuanbao|email|mattermost|matrix|dingtalk|wecom|homeassistant|whatsapp|irc|ntfy|simplex|buzz|photon],
#  plus webhook platforms: whatsapp_cloud/msgraph/webhook/bluebubbles/feishu/sms/teams/line/google_chat/raft/a2a)
# the Tauri desktop app (desktop/) and any browser dashboard talk to this
# gateway over HTTP/SSE (local-app CORS is built in; see desktop/README.md)
curl -H "Authorization: Bearer $ULNCLAW_GATEWAY_KEY" \\
     -H "Content-Type: application/json" \\
     -d '{"messages":[{"role":"user","content":"Hello!"}]}' \\
     http://127.0.0.1:8642/v1/chat/completions
```

Example `~/.ulnclaw/config.toml`:

```toml
# timezone = "Asia/Shanghai"   # IANA zone for prompt timestamps
                               # (ULNCLAW_TIMEZONE / HERMES_TIMEZONE env override)

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
# mode = "manual"              # manual | smart (aux-LLM guardian) | off
# cron_mode = "deny"           # deny | approve — unattended cron runs
# smart_policy = ""            # operator rules for the smart-approval guardian
# denial_breaker_threshold = 3  # consecutive guardian DENYs before the hard-stop escalation
# deny = []                     # fnmatch globs that always block (even mode = "off")

[delegation]
max_concurrent_children = 3

# [[mcp.servers]]
# name = "filesystem"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me"]
# lazy = false        # true = register from the schema cache, spawn on first call
# [mcp.servers.env]   # stdio children only see filtered env: safe baseline +
# API_TOKEN = "${MY_TOKEN}"  # XDG_* + ~/.ulnclaw/.env keys + this block;
#                      # ${VAR}/${env:VAR} interpolate from the secret scope first

# Remote MCP server (Streamable HTTP by default; transport = "sse" for the
# legacy SSE protocol; headers ride on every request):
# [[mcp.servers]]
# name = "remote"
# url = "https://mcp.example.com/mcp"
# # transport = "sse"
# [mcp.servers.headers]
# Authorization = "Bearer sk-..."

# OAuth-protected remote server (browser-based OAuth 2.1 + PKCE on first
# use; tokens cached under mcp-tokens/, refreshed automatically):
# [[mcp.servers]]
# name = "oauth-remote"
# url = "https://mcp.example.com/mcp"
# auth = "oauth"
# [mcp.servers.oauth]
# # client_id = "pre-registered-id"   # skip dynamic registration
# # scope = "read write"

[gateway]
host = "127.0.0.1"
port = 8642
# key = "sk-..."        # bearer token; env ULNCLAW_GATEWAY_KEY overrides
# multiplex_profiles = false  # true = serve /p/<profile>/... mirrors, each
                              # backed by its [profiles.<name>] override and
                              # its own fail-closed secret scope
                              # (profiles/<name>/.env; unscoped credential
                              # reads error instead of leaking across profiles)

# [terminal]
# backend = "docker"    # "local" (default) | "docker" | "ssh"
# container = "ulnclaw-dev"
# image = "ubuntu:24.04"
# env_passthrough = ["TENOR_API_KEY"]   # vars allowed past the sandbox credential scrub

# [tool_output]
# max_bytes = 100000        # terminal output head+tail cap
# max_lines = 2000          # read_file pagination cap
# max_line_length = 2000    # per-line clamp ('... [truncated]')

# [security]
# allow_private_urls = false  # true lets web tools fetch private/internal IPs
#                             # (cloud metadata endpoints stay blocked either way;
#                             #  env ULNCLAW_ALLOW_PRIVATE_URLS also works)

# [checkpoints]
# enabled = true        # transparent snapshots before write_file/patch

# Auxiliary model routing: run secondary calls on a different model.
# [auxiliary.compression]   # context-compression summaries
# provider = "openai"
# model = "gpt-5.2-mini"
# [auxiliary.vision]        # image analysis (vision_analyze / browser_vision)
# model = "gpt-5.2"
# [auxiliary.title_generation]  # session titles after the first exchange
# enabled = true            # kill switch (is_truthy_value semantics)
# language = ""             # pin title language; blank matches the user

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
- Design notes: [Multiplexing Gateway](docs/design/multiplexing-gateway.md) (multi-profile routing + fail-closed secret scopes) · [Browser CDP Client](docs/design/browser-cdp.md) (CDP session layer + `/v1/browser/*` control plane)

### Building & Testing

```bash
cargo test                     # 990 tests
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
[对标矩阵](docs/zh/hermes-parity.md) —— 与 hermes-agent v2026.8.3 的核心
对齐已完成，含消息平台网关（Telegram/Discord/Slack/Signal/微信/QQ/元宝/邮件/Mattermost/Matrix/钉钉/企微/飞书/Home Assistant/SMS (Twilio)/WhatsApp (Baileys 桥)/IRC/ntfy/SimpleX/Teams/LINE/Google Chat/Buzz/Photon (iMessage)/Raft/A2A）、插件与 shell 钩子
体系、Secrets 保险库、computer-use、OAuth 登录 + 技能同步 + 本地 OAuth 上游代理，以及 Tauri
桌面 GUI（`desktop/`）。

### 核心特性

- **🤖 Agent 循环** —— 工具调用、迭代预算、用量统计、记忆注入、步骤/工具回调
- **🔧 50+ 内置工具** —— terminal/process、文件读/写/补丁/搜索、web 搜索/抽取、经 xAI 的 X（Twitter）搜索（`x_search`，可选工具集 + `XAI_API_KEY`）、视频理解（`video_analyze`，可选 `video` 工具集）、记忆、todo、会话搜索、clarify、技能、委派、execute_code、cronjob、视觉、图像生成、视频生成（`video_generate` 注册表 —— BFL FLUX 3 经 Nous 工具网关、xAI Imagine 含 edit/extend、FAL 六家族队列、DeepInfra）、桌面项目（`project_list/create/switch`，可选 `project` 工具集）、Discord 服务器工具（`discord`/`discord_admin` —— Bot 令牌、意图门控 schema、`server_actions` 白名单）、飞书/Lark 文档工具（`feishu_doc_read` + `feishu_drive_*` 评论工具 —— 应用凭据经 env/secret scope/`[messaging.feishu]`）、Spotify 工具（7 个 `spotify_*` 工具 —— 播放/设备/队列/搜索/歌单/专辑/资料库，经 PKCE OAuth，`ulnclaw spotify-auth login`）、元宝工具（5 个 `yb_*` 工具 —— 群信息/成员、私信、贴纸搜索/发送，经存活适配器）、跨频道消息（`send_message` —— 经频道目录 + 主频道在所有已连接平台 send/list/react/unreact，Telegram/Discord/Slack 原生 `MEDIA:<path>` 附件，表情回应支持最近消息回退）、学习时间线（`ulnclaw journey`）、TTS、Home Assistant、kanban、工具搜索
- **🧰 工具集** —— hermes 兼容分组（`coding`、`web`、`file`、`safe`、`debugging`……），支持组合与启用/禁用策略
- **🛡️ 审批系统** —— 命令归一化、硬性底线（自动阻止）、高成本操作先确认再执行；REPL 提示 + 网关 HTTP 运行审批（fail-closed 超时、`always` 授权持久化）；tirith 执行前内容扫描（退出码裁决、SHA-256/cosign 校验自动安装、fail-open 熔断器）
- **💾 SQLite 状态库** —— 会话/消息 FTS5 全文检索、会话血缘（父子会话）、定时任务、kanban 看板；受损数据库的离线非破坏性 `sessions recover`（rowid 抢救、孤儿会话重建、FTS 重建）
- **🗜️ 上下文压缩** —— 预算触发，中段对话经二次模型调用摘要，外加三层工具结果持久化：超大工具输出（>10 万字符，`read_file` 豁免）经 terminal backend 存入 `ulnclaw-results/` 并替换为预览 + 路径，单轮 20 万字符聚合预算优先溢出最大结果
- **🤝 委派** —— 并行子代理，隔离上下文，深度限制；顶层委派后台即发即忘（实时记录在 `cache/delegation/live/`），整批完成后以单条汇总结果重回会话；派发与结果持久化于 SQLite 委派登记表，完成的工作可跨重启保留（崩溃后仍在运行的委派以终态 "outcome unknown" 报告恢复投递）
- **🧬 混合智能体（MoA）** —— `[moa]` 预设将提示词并行扇出给参考模型，经聚合器综合（`ulnclaw moa run/list/delete`、REPL `/moa`）；设置 `[model] provider = "moa"` 可让整个 agent 循环跑在预设上（持久门面：按轮参考缓存、`save_traces` JSONL trace、`privacy_filter` PII 脱敏）
- **🗺️ 模型目录** —— models.dev 多 provider 清单，三级缓存（内存 → 磁盘 → 网络失败退避 5 分钟）：`ulnclaw models providers|list|info|refresh`、网关 `/api/model/options` 多 provider 选择器清单 + `?refresh=true`、`ULNCLAW_MODELS_DEV_URL` 镜像覆盖
- **⏰ 定时任务** —— `30m` / `every 2h` / `0 9 * * *` / ISO 一次性计划 + 轮询调度器
- **📐 蓝图（Blueprints）** —— frontmatter 声明 `metadata.hermes.blueprint.schedule` 的技能可排程（`skills blueprints`、`skills schedule/unschedule`）
- **🛡️ 技能守卫** —— `skills scan <name>` 运行 `skills-guard-v1` 静态扫描器（119 条威胁模式、不可见 Unicode 与结构检查、来源信任等级），在安装/运行第三方技能前把关；dangerous 技能即使来自受信任仓库也会被拦截
- **🔌 MCP 客户端** —— stdio JSON-RPC + 远程 Streamable HTTP / SSE 传输（`url` + 可选 `transport = "sse"` + 静态 `headers`；`auth = "oauth"` 运行 OAuth 2.1 + PKCE（元数据发现、动态注册、loopback 回调、令牌刷新、401 恢复））：任意 MCP 服务器的工具以 `mcp__<server>__<tool>` 注册；npx/uvx 启动前经 OSV 恶意软件检查（MAL-* 通告阻止、fail-open）；`lazy = true` 的服务器直接从磁盘 schema 缓存注册、不拉起子进程，首次工具调用才启动（hermes 懒启动）；REPL `/reload-mcp` 从最新配置重连服务器，带提示词缓存失效确认（once/always/cancel）；stdio 子进程运行于过滤后的环境（仅安全基线 + `XDG_*` + `<home>/.env` 键 + 声明的 `env`——环境中的密钥不会泄漏），`${VAR}`/`${env:VAR}` 占位符优先从密钥作用域解析；网关为 dashboard 中介 MCP OAuth：`POST /api/mcp/servers/<name>/auth` 启动流程并返回待打开的授权 URL，浏览器重定向落到开放的 `GET /api/mcp/oauth/callback/<server>` 路由（state 校验），`GET /api/mcp/oauth/flows/<id>` 报告状态与发现的工具；401 恢复按失败令牌去重，其他进程在磁盘上刷新的令牌于下一请求经 mtime 监视拾取；`/reload-mcp` 亦可在平台聊天中使用——适配器支持时渲染原生按钮，否则以 `/approve`/`/always`/`/cancel` 文本回复
- **📡 MCP 频道桥** —— `ulnclaw mcp serve [--verbose]` 经 stdio JSON-RPC 向任意 MCP 客户端（Claude Code、Cursor、Codex 等）暴露消息会话：跨全部平台会话的 `conversations_list`/`conversation_get`/`messages_read`/`attachments_fetch`，`events_poll`/`events_wait` 长轮询事件流（200 ms mtime 门控 DB 轮询、启动不重放），经 `send_message` 管道的 `messages_send`（含无存活网关时的 Telegram/Discord/Slack 独立 REST 投递），频道目录目标的 `channels_list`，以及桥会话审批的 `permissions_list_open`/`permissions_respond`
- **✍️ ACP 适配器** —— `ulnclaw acp [--verbose]` 将 ulnclaw 作为 Agent Client Protocol stdio 服务器运行于 Zed 等编辑器：带历史回放的会话管理、流式 `session/update` 通知（消息/思考块、带 kind 映射的工具调用、`todo` 原生 plan 更新）、多模态提示（图像块走原生视觉通道）、协作式取消，以及经 `session/request_permission` 编辑器弹窗呈现的工具审批
- **📦 批量运行器** —— `ulnclaw batch --dataset-file data.jsonl --run-name my_run [--batch-size N] [--resume]` 以并行工作池在 JSONL 提示词数据集上运行 agent，支持检查点续跑（内容扫描 + 索引检查点）、每批 hermes from/value 轨迹文件、工具用量/推理统计聚合与最终 `summary.json`
- **📨 Send CLI** —— `ulnclaw send --to telegram "部署完成"`（或管道 stdin、`--file`、`--subject`、`--list [platform]`、`--json`、`--quiet`）从脚本/cron/CI 投递消息，无 LLM、bot 令牌平台无需运行网关——hermes 退出码契约（0/1/2）
- **🌍 浏览器自动化** —— CDP WebSocket 客户端 + 内置监督器（自动启动无头 Chrome/Chromium，或用 `ULNCLAW_BROWSER_CDP` 指向自有浏览器）：带元素引用的可访问性快照、点击/输入/滚动/按键/截图/执行 JS/对话框
- **🌐 浏览器自动化** —— 12 个 `browser_*` 工具，基于 CDP WebSocket 客户端（带元素引用的可访问性快照、点击/输入/滚动/按键、截图 + 视觉、console/eval、原始 CDP、对话框）；托管无头 Chrome（`ULNCLAW_BROWSER_CDP=auto`）、任意已有 DevTools 端点、Camofox 反检测 REST 后端（`CAMOFOX_URL`），或经 Browserbase / Browser Use / Firecrawl 按需云会话（`[browser] cloud_provider`），内置 hermes 级 SSRF 防护（元数据底线、私网地址门控、重定向复检、原始 CDP 白名单）并对浏览器输出强制脱敏
- **🚪 HTTP 网关** —— `ulnclaw gateway`：OpenAI 兼容 `/v1/chat/completions` + `/v1/responses`（会话续接）、两者均支持 `stream: true` SSE 流式（令牌增量、工具进度/函数调用事件）、带 SSE 事件 + 审批处理的异步 `/v1/runs`、会话 API（含 `PATCH`/fork + 逐轮生效的会话级模型锁）、`/api/jobs` 定时任务管理（增删查改 + pause/resume/run，内置调度器自动执行到期任务，外部投递目标（`deliver`：`origin`/平台名/`platform:chat[:thread]`/`all`，按平台发送器 + home 频道环境变量解析，`[SILENT]` 抑制、包装抬头与失败摘要——`GET /api/jobs/delivery-targets` 列出可用目标）与 Chronos NAS 触发 webhook `POST /api/jobs/fire`（经 `[cron.chronos]` JWT 校验，202 + 后台运行）、`/v1/skills` + `/v1/toolsets` 发现端点、`/api/model/options`（多 provider 选择器清单）、Prometheus `/metrics`、令牌核算 `/api/usage`、后台委派登记 `/v1/delegations`、浏览器 CDP 实时控制 `/v1/browser/status|connect|disconnect`、无内容 OTLP 健康/诊断导出的网关监控（`[monitoring]`，`ulnclaw monitoring status`）、Bearer 鉴权，以及单实例 pidfile 守卫（`gateway.pid` 含防 PID 复用的启动时刻令牌——二次启动被拒绝，`--replace` 接管运行中的实例，`--force` 并行运行）
- **🖥️ 终端环境** —— `terminal` 可在本地（默认）、docker（自动创建容器）或 ssh 上执行（`[terminal] backend`）
- **🩺 终端失败提示** —— 失败命令自带可执行提示：良性退出码会被解释（`grep=1` → "无匹配（非错误）"，`exit_code_meaning`），常见失败形态附加一条恢复建议（`hint`）：命令/模块未找到、git 冲突、gh 字段漂移与限流、权限错误、退出码 124/126/137
- **🔬 环境探针** —— 终端后端为本地时，向系统提示注入一行确定性的 Python 工具链说明（pip/python3 版本错配、缺 pip 模块、PEP 668、缺少裸 `python`）；健康环境保持静默，后台探测 + 超时即放行（`[agent] environment_probe`）
- **🖥️ 桌面桥接工具** —— 面向 GUI 宿主的 `close_terminal` / `read_terminal` / `focus_pane` / `open_preview` / `react_to_message`（表情回应，`[display] message_reactions`）：`ULNCLAW_DESKTOP=1` 门控，经 `desktop` 桥接层路由（宿主安装的发射器按 UI 会话收到 `terminal.close` / `pane.reveal` / `preview.open` 事件）；从不杀进程，未接入宿主时返回 "desktop only"；Tauri 外壳经 `/api/desktop/events` SSE 桥接接通（P231），`read_terminal` 走 HTTP 往返
- **🧹 ANSI 剥离** —— terminal/execute_code 输出在送达模型前清除 ECMA-48 转义序列（颜色、光标移动、OSC 标题、8-bit C1），转义序列不会泄漏进上下文或文件写入
- **🔒 沙箱凭证清洗** —— terminal/execute_code 子进程的环境中剥离 provider 与工具凭证（hermes GHSA-rhgp-j443-p4rf 语义）；技能 `required_environment_variables` 与 `[terminal] env_passthrough` 允许其余变量通过——provider 凭证永不可被放行
- **🚫 二进制守卫** —— `read_file` 拒绝约 80 种二进制扩展（图像、压缩包、可执行文件、字体、字节码、数据库），并提示改用 vision_analyze/terminal；`.pdf` 保持可读
- **📏 可配置输出上限** —— `[tool_output] max_bytes/max_lines/max_line_length` 无需改源码即可调整 terminal 截断、read_file 分页与每行截断上限
- **🕵️ 密钥脱敏** —— 约 55 种厂商密钥前缀、JWT、私钥、数据库连接串、认证头与 env 转储的 `KEY=value` 在输出送达模型前脱敏；文件内容使用不可复用哨兵，截断密钥永不会被写回
- **📸 检查点** —— 文件编辑前的透明 git 快照（共享 shadow 存储、按项目快照链），`ulnclaw checkpoints list/restore/diff/prune`
- **📝 工作区 diff** —— `ulnclaw diff [--staged|--all]` 显示 git 工作区变更（含未跟踪文件），REPL `/gitdiff`
- **🌐 Provider** —— OpenAI 兼容端点（OpenAI、OpenRouter、DashScope、Ollama、llama.cpp）+ 原生 Anthropic Messages API provider（tool_use/tool_result 块、SSE 流式、OAuth bearer），本地 provider 免密钥；按任务辅助路由（`[auxiliary.compression]`、`[auxiliary.vision]`、`[auxiliary.title_generation]`）可将二次调用发往不同 provider/模型；`[model] fallbacks` 回退链（按轮恢复主 provider）；用户自定义 `[providers.<slug>]` 条目（base_url/api_key/key_env/model/mode）与 env 密钥认证的规范 provider、配置提示骨架行一同出现在 `/api/model/options` 多 provider 选择器清单中（`[model_catalog] excluded_providers` 隐藏行）
- **📡 消息平台** —— Telegram/Discord/Slack/Signal 适配器运行于 `ulnclaw gateway` 内（`[messaging.*]`，Signal 经 signal-cli HTTP 守护进程，Slack 输入状态经 `assistant.threads.setStatus`，Telegram clarify 内联键盘 + callback_query 点按路由、Discord clarify 按钮 + INTERACTION_CREATE 路由、Slack Block Kit clarify 按钮 + block_actions 路由），图片附件以原生多模态内容注入回合（P226 —— ≤ 8 MB 的 `image/*` base64 `data:` URL，OpenAI 兼容/Anthropic provider；`[messaging] multimodal_injection = false` 保留路径引用流），WhatsApp Cloud（`[messaging.whatsapp_cloud]` + `ulnclaw whatsapp-cloud` 凭据向导，带字段形状校验；P271）与 Microsoft Graph 接入挂载为网关 webhook 路由（`/webhooks/whatsapp` HMAC 校验、`/webhooks/msgraph` clientState 校验 + 回执去重、资源过滤与提示词模板），另有通用签名 webhook 平台（`[messaging.webhook]` 路由 `/webhooks/hook/<name>` —— Svix/GitHub/GitLab/HMAC-V2 签名方案、每路由限流、投递 id 幂等、`deliver_only` 零 LLM 推送），并支持 CLI 管理的动态订阅：`/webhooks/<name>` 每次请求热加载 `webhook_subscriptions.json`（`ulnclaw webhook subscribe|list|remove|test`，密钥存储 0600 权限；P270）、BlueBubbles iMessage（`[messaging.bluebubbles]` → `/webhooks/bluebubbles` 密码校验 webhook、LRU 缓存的 chat-GUID 解析、REST 文本/附件发送）、微信个人号（`[messaging.weixin]` —— 腾讯 iLink Bot API 长轮询、`ulnclaw weixin login` 扫码登录、双向 AES-128-ECB 加密 CDN 媒体、context_token 回显发送）、QQ（`[messaging.qq]` —— 官方 QQ Bot API v2 WebSocket 网关 + REST、markdown 回复、分块媒体上传、`asr_refer_text` 语音转写、内联键盘执行审批 + INTERACTION_CREATE 路由、QR 扫码配置引导（`ulnclaw qq login`）+ 含 DM 策略与 home 频道选择的完整设置向导（`ulnclaw qq setup`））、元宝（`[messaging.yuanbao]` —— 腾讯元宝 App 机器人 WS 网关 + 手写 protobuf 线格式编解码、HMAC-SHA256 sign-token 认证、markdown 感知分块文本回复、COS 上传出站图片/文件媒体、表情包（`STICKER:` 回复标签发 TIMFaceElem、模糊贴纸查找、入站 `[emoji: <名称>]` 渲染）、入站图片/文件媒体解析下载入媒体缓存、引用/观察媒体回填 + `[Replying to: …]` 引用提示、微信转发聊天记录深解析（elem_type 1009））、邮件（`[messaging.email]` —— IMAP 轮询 + SMTP 回复 `Re:` 线程、SPF/DKIM/DMARC 发件人认证）、Mattermost（`[messaging.mattermost]` —— REST v4 + WebSocket 事件、提及门控、线程回复、文件上传/下载）、Matrix（`[messaging.matrix]` —— 原生 Client-Server API `/sync` 循环、mxc 双向媒体、不支持端到端加密）、钉钉（`[messaging.dingtalk]` —— 手写 Stream Mode WS、sessionWebhook markdown 回复、downloadCode 媒体、🤔Thinking → 🥳Done 表情回应、配置 `card_template_id` 时经 card_1_0 的 AI 流式卡片）、企业微信（`[messaging.wecom]` —— AI Bot WS 网关、respond-msg markdown、分块媒体上传、客户端文本切块批处理）、飞书（`[messaging.feishu]` —— lark_oapi WebSocket 长连接 + 手写 protobuf 帧编解码（hermes 默认 `connection_mode = "websocket"`），或网关 webhook `/webhooks/feishu` 签名校验事件、租户令牌媒体、Typing/CrossMark 处理状态表情 + 入站表情路由、交互式审批/更新确认卡片 + webhook 卡片动作路由（非审批点击转 `/card` 合成命令）、云文档评论 agent + 会议邀请卫星处理器）、Home Assistant（`[messaging.homeassistant]` —— WebSocket state_changed 事件流、默认关闭的 watch 过滤 + 按实体冷却、回复以持久通知送达、仅凭凭证即注册独立 `notify/notify` 发送器）、SMS/Twilio（`[messaging.sms]` —— 网关 webhook `/webhooks/twilio`、X-Twilio-Signature HMAC 校验、去 markdown 1600 字符 REST 回复）、WhatsApp（`[messaging.whatsapp]` —— 网关内置 Baileys 桥（`scripts/whatsapp-bridge/`）并由网关监督：npm 依赖哈希印章、pidfile/端口陈旧清理、`bridge.log` 拉起、两阶段就绪 + `scriptHash` 陈旧握手；`/health`/`/messages` 轮询、`/send` + `/send-media` + `/send-poll` + `/send-location` + `/edit`（原生投票 + 投票式 clarify、位置图钉、消息编辑）、已读回执、self-chat `[owner reply]` 准入）、IRC（`[messaging.irc]` —— rustls TCP 客户端、NickServ 支持、频道寻址门控）、ntfy（`[messaging.ntfy]` —— 主题流订阅 + REST 发布、回声标签防循环）、SimpleX（`[messaging.simplex]` —— simplex-chat 守护进程 WS 客户端、自动接受联系人、语音/文档媒体）、Teams（`[messaging.teams]` —— 网关 webhook `/webhooks/teams`，原生 Bot Framework 协议、OAuth2 客户端凭证发送、AdaptiveCard 执行审批按钮 + 默认拒绝点按门控）、LINE（`[messaging.line]` —— 网关 webhook `/webhooks/line`，X-Line-Signature 校验、reply token + push 回退、慢 LLM postback 按钮、令牌门控 `/line/media` HTTPS 出站媒体）、Google Chat（`[messaging.google_chat]` —— 网关 webhook `/webhooks/googlechat` Google ID 令牌校验或 Pub/Sub REST 拉取入站、服务账号 RS256 JWT Chat API 发送、按用户 OAuth 原生附件投递（聊天内 `/setup-files` + `ulnclaw google-chat-oauth`）、打字指示卡片原地改写为回复）、Buzz（`[messaging.buzz]` —— Block Nostr 协作平台：NIP-42 认证 WebSocket 订阅（签名 kind-22242 认证，hermes `nostr_auth` 移植）+ CLI 轮询回退、kind-9 聊天事件、提及门控、自身公钥回声抑制、👀 已读 tapback、启动高水位播种（历史永不重放）+ `dms list`/`channels list` DM 发现、kind-44100 成员事件实时 DM 再发现 + 动态 `hermes-buzz-dm-<n>` 订阅、p 标签 DM 闩锁分类）、Photon（`[messaging.photon]` —— iMessage，经 Photon Spectrum sidecar HTTP API：`/healthz` 门控、类型化 NDJSON 入站流、富链接渲染 + 预览图/回声抑制、群组唤醒词提及门控、`/typing` 输入指示、纯 URL `/send-richlink` 回复、白名单 ∪ 配对门控、可选 `PHOTON_REACTIONS` 生命周期 tapback + 入站 tapback 路由）、Raft（`[messaging.raft]` —— 网关 wake 端点 `/webhooks/raft/wake`：bridge 令牌校验的 `raft-activity.v1` 事件 + 自动拉起 `raft agent bridge` 子进程）与 A2A（`[messaging.a2a]` —— Agent2Agent v1.0 服务端：`/.well-known/agent-card.json` 发现 + JSON-RPC `POST /a2a` message/send + 任务台账）：白名单配对（fail closed）+ hermes 风格交互式配对码（`pairing list/approve/revoke/clear-pending`）、媒体附件缓存于 `media-cache/` 并以路径引用交付（出站 `MEDIA:` 标签在 Telegram/Discord/Slack 原生上传；WhatsApp 媒体双向经 Graph `/media` 端点）、每聊天一条持久会话、hermes 风格回复分块。`clarify` 工具在聊天中可用：WhatsApp 渲染原生按钮/列表、Telegram 渲染内联键盘、Discord/Slack 渲染按钮，其余平台编号文本；点按与后续文本应答待决提问。Slack 支持原生斜杠命令：`ulnclaw slack manifest` 生成应用清单，把每个平台命令注册为一等斜杠（assistant/agent/扁平 DM 三种体验），斜杠信封走常规分发路径并以 `response_url` 回复；所有平台直答命令集（`/help` `/skills` `/tools` `/recap` `/title` `/usage` `/insights`）免 LLM 回合（P265）
- **🎙️ 语音转写（STT）** —— 入站语音/音频消息在 agent 回合前转写（`[stt]` 配置）：内置 `local_command` / `groq` / `openai` / `mistral` / `xai` / `elevenlabs` / `deepinfra` provider，另支持自定义 `[stt.providers.<name>]` 命令 provider，转写文本以 🎙️ 消息回显并注入回合（复刻 hermes 回退/哨兵语义）；`transcribe_audio` 工具（可选 `stt` 工具集）覆盖任意音频文件。Python 专属的 faster-whisper `local` provider 以 `stt.local.command` / 云后端替代
- **🐾 宠物（petdex）** —— `ulnclaw pets list|install|select|show|off|scale|remove|doctor|hatch`：领养 petdex 动画吉祥物（数千个的在线画廊、主机锁定的 `<home>/pets/` 安装），在终端以 kitty/iTerm2/sixel 图形协议或真彩半块回退播放动画，`[display.pet]` 单一 scale 旋钮同步缩放所有界面 —— 或用 `hatch` 从文字描述孵化全新宠物：LLM 基础草稿 → 锚定动画行 → 切片/归一化精灵图 → 自动领养（OpenAI 兼容图像端点，`[pets]` 配置）
- **📋 Kanban 引擎** —— `ulnclaw kanban`：`kanban.db` 中的多看板任务引擎（hermes 状态 todo/ready/running/scheduled/blocked/done/archived，带图标），带 TTL 的认领锁 + 过期接管与心跳、过期 run 完成守卫（worker 携带自身 run id，被回收的 attempt 无法完成/阻塞新 attempt）、类型化阻塞（`kanban block --kind dependency|needs_input|capability|transient` —— dependency 停在 todo 等父任务完成；同因反复阻塞升级进 triage）、父→子任务依赖、评论与事件轨迹、看板增删改查，以及调度器（`kanban dispatch [--max-spawn N] [--dry-run]` + `POST /api/kanban/dispatch`）：回收过期认领、崩溃（pid 已死）与心跳停滞的 worker、晋升父任务已完成的 todo（父任务完成后自动恢复非粘性阻塞任务，达到失败上限者除外）、跳过指派给未配置 profile 的就绪任务（`skipped_nonspawnable` 认领拉取通道）、遵守 `[kanban] max_in_progress_per_profile`（#21582 按 profile 在飞上限）与 `[kanban] max_in_progress` 全局上限（#33488 —— 运行列已满时本轮直接跳过，否则补齐到上限），整个安装内首次物化 scratch 工作区时一次性告警（`tip_scratch_workspace` 事件 + `.scratch_tip_shown` 哨兵），以及 `create --initial-status blocked` 建卡即停入待审、工作流模板钩子（`workflow_template_id`/`current_step_key` 列与 `list --workflow-template-id` 过滤）、按任务 model/provider worker 钉选（`create --model M --provider P`、`set-model [--provider P]` —— spawn 的 worker 携带全局 `-m/--provider` 标志），以及 goal 模式 worker（`kanban create --goal [--goal-max-turns N]` —— spawn 的 worker 在同一会话内循环，直到辅助评判模型认可卡片完成或 turn 预算耗尽阻塞；goal 卡片的完成在 `kanban done` / `kanban_complete` 上过评判门）、每个 tick 在非阻塞 `.dispatch.lock` 下进行（#35240 —— 第二个调度器跳过本 tick 而不是竞态写库）、为就绪任务生成分离的 `ulnclaw run` worker —— 每个 worker 的工作区在创建时选定（`kanban create --workspace scratch|worktree|worktree:<path>|dir:<path>` 与 `--branch`，调度器 spawn 前解析并持久化；`[kanban] worktrees` 保留旧版 worktree 默认；`kanban gc` 清理已完成任务的树），以及 swarm 编排器（`kanban swarm <goal> --worker ASSIGNEE:TITLE --verifier X --synthesizer Y`：根黑板任务 + 并行 worker + 验证者 + 综合者，调度器随父任务完成逐级晋升）、triage 流水线（`kanban create --triage` 暂存想法，`kanban specify` / `kanban decompose` 经辅助 LLM 细化或扇出为路由到 profile 的子任务图 —— 网关调度器每 tick 自动分解新 triage 任务，`[kanban] auto_decompose` 实时重读作为安全开关）、只读看板体检（`kanban diagnostics`）、网关通知订阅（`kanban notify-subscribe / notify-list / notify-unsubscribe` —— 网关通知器循环把终态事件（✔ 完成 / ⏸ 阻塞 / ⏱ 超时……）经消息平台投递到订阅的聊天，并在 agent 建任务时记录 `session_id` 后以 `[kanban] Task …` 回合唤醒创建者会话）、按任务的 worker 日志查看（`kanban log [--tail N]`，按 `[kanban] worker_log_rotate_bytes`（默认 2 MiB）轮转并保留一份备份代）、重生守卫对立即重试无益的就绪任务延后重生（限流冷却、配额/鉴权阻塞、近期已成功、已有 PR —— 每次延后记录 `respawn_guarded` 事件）、按尝试的运行历史（`kanban runs [--json]`，每次 认领→完成/阻塞/回收/超时 尝试一行 `task_runs` 记录）、含历史尝试与父任务交接的完整 worker 简报（`kanban context`，`kanban_show` 工具同步提供）、结构化完成交接（`kanban done --summary … --metadata '{…}'` 写入收尾 run）、完成工件（`kanban done --artifact PATH…` —— scratch 工作区文件在清理前暂存到 `kanban/attachments/<task>/`，25 MiB 上限）、review 列（`kanban review <id> [--reason]` 在开 PR 后把运行中的任务停入 review；调度器 spawn 评审 agent —— 装有 `sdlc-review` 技能时强制加载 —— 认领时不再复查父任务依赖）、批量生命周期命令（`done/block/schedule/unblock/promote/archive` 支持多 id，`archive --rm` 清除已归档任务，`promote --dry-run --json`）、完成态恢复（`kanban edit --result/--summary/--metadata` 改写已 done 任务的交接；block 先写 `BLOCKED: <reason>` 评论再落锁）、完成反幻觉门（`kanban done --created-card ID…` 在任何改动前核验声明的卡片；幻影 id 以 `completion_blocked_hallucination` 事件阻断完成，完成后对无法解析的 `t_<hex>` 文本引用加以标记）、数据库自愈（`kanban repair`）、`kanban assignees`，以及一等项目登记簿（`ulnclaw project create/list/show/add-folder/remove-folder/rename/set-primary/use/archive/restore/bind-board` —— `projects.db` 中的具名多文件夹工作区，带活跃项目指针；`kanban create --project <id|slug>` 把卡片 worktree 锚定到项目主仓库下（`<repo>/.worktrees/<task-id>`）并派生确定性分支 `<slug>/<task-id>[-<title-slug>]`，`project bind-board` 把主仓库镜像为绑定 board 的 `default_workdir`；登记簿经网关 `/api/projects/*` 增删查改与 `scan`/`repos` 发现端点向桌面开放）；生命周期流转触发 `kanban_task_*` 插件钩子。一块看板、三个界面：agent `kanban_*` 工具与桌面看板挂件（网关 `/api/kanban/*`）共享同一引擎与数据库
- **🔌 插件与钩子** —— 目录插件（`~/.ulnclaw/plugins/<name>/plugin.toml`：hooks + 子进程工具）与 `[hooks]` 配置式 shell 钩子，复刻 hermes 首次使用同意机制（`plugins list/install/update/remove/enable/disable/accept-hooks` —— install/update/remove 管理 git 托管插件、`hooks list/test/revoke/doctor`）；核心触发 hermes 运行期实际发出的全部 13 个钩子事件（工具/LLM 前后、API 请求生命周期、会话边界、网关分发门控）
- **🔑 Secrets 保险库** —— 外部秘密源在启动时、provider 读取 env 之前应用（`secrets status/sync`）：command 助手、Bitwarden Secrets Manager（`bws` —— 固定版本自动安装、AES-GCM 加密 TTL 缓存、`secrets bitwarden setup` 向导）、1Password（`op://` 引用、`secrets onepassword setup/set`），完整复刻 hermes 优先级语义
- **🛡️ 出站防火墙** —— Docker 沙箱的托管 iron-proxy（`egress install/setup/start/stop/restart/reload/status/disable/config`、`/egress` 状态）：沙箱只见铸造的代理令牌 —— 真实 provider 密钥由守护进程在白名单主机上换入，永不越界；固定 v0.39.0 二进制经 SHA-256 + GPG 验签、openssl CA、失败关闭的令牌规则、管理 API 热重载（hermes `hermes egress`）
- **🖱️ Computer use** —— `computer_use` 工具经 cua-driver 守护进程（MCP over stdio，完整 hermes schema），与 hermes 相同的审批门控；`computer-use status/doctor/install`
- **🔄 OAuth + 技能同步** —— `auth login` 对任意 `[oauth]` provider 执行 RFC 8628 设备流；`sync status/pull/push/now` 经 HTTP(S) 或共享目录同步技能；`proxy start` 运行本地 OpenAI 兼容代理（`127.0.0.1:8645/v1`），把存储的 OAuth bearer（自动刷新）附加到配置的 `[proxy] upstream_url`，让外部应用复用订阅
- **🖥️ 桌面 GUI** —— `desktop/`：Tauri 2 外壳（取代 hermes 的 Electron 应用），承载聊天界面（会话重命名/删除悬停操作、`hermes.tool.progress` SSE 事件驱动的实时工具进度条、网关命令 + 已装技能的 `/` 斜杠补全弹层、`hermes.tool.started/completed` SSE 事件驱动的可展开工具调用卡片、剪贴板图片粘贴经 `POST /api/uploads` 上传并以媒体路径引用附加）看板挂件、项目视图（第三个标签页，基于 `/api/projects/*`：项目卡片支持创建/启用/归档/删除、文件夹管理与主目录切换、board 绑定、文件系统仓库扫描与发现仓库收养；会话按 cwd 前缀匹配 `projects.db` 得到所属项目 slug 并以侧栏徽章显示）、任务视图（第四个标签页，基于 `/api/jobs`：定时任务行带状态、调度、技能与下次运行倒计时，支持暂停/恢复、立即运行、行内编辑、删除与创建对话框，10 秒轮询）、用量视图（第五个标签页，基于 `/api/usage`：令牌核算仪表盘——进程/存储汇总卡片：网关令牌、工具调用、API 请求、异步运行——加按会话表格与按比例令牌条，另含基于 `/api/insights` 的洞察区块（7/30/90 天窗口：热门模型/工具/会话、活跃天数与估算费用），10 秒自动刷新）、配置视图（第六个标签页，基于 `/api/config`：扁平化 config.toml 编辑器——密钥打码、增删键、JSON 标量解析、待保存变更徽标——并列出 .env 键名）、诊断视图（第七个标签页，基于 `/api/doctor`：运行 `ulnclaw doctor` 检查，按分组显示 ✓/⚠/✗/ℹ 行与问题面板，可选在线 provider 连通性探测，另含基于 `/api/monitoring` 与 `/v1/browser/status` 的监控与浏览器 CDP 面板，显示 OTLP 健康导出状态、浏览器配置状态，并经 `/api/logs/tail` 实时拖尾网关日志（可按级别过滤）、经 `/api/mcp/servers` 显示已配置 MCP 服务器的传输方式与 OAuth 状态，未授权的 OAuth 服务器可一键发起连接（`POST /api/mcp/servers/:name/auth`），呈现授权页面链接并轮询流程直至授权完成，另含看板诊断面板：各板进行中/总数计数、当前板状态直方图与受阻任务清单）、Webhooks 视图（第八个标签页，基于 `/api/webhooks/subscriptions`：列出动态 webhook 路由，支持复制 URL、签名测试触发与删除，创建/更新表单对应 `ulnclaw webhook subscribe`）、运行视图（第九个标签页，基于 `/v1/runs`：跟踪的异步运行，带状态徽章、结果查看、停止，等待中的运行可实时批准——一次/本会话/始终/拒绝）、技能视图（第十个标签页，基于 `/v1/skills` + `/v1/toolsets`：已安装技能目录带分类/描述/源路径，工具集启用状态卡片带可展开工具列表；标签栏自动换行）、会话记录视图（第十一个标签页，基于 `/api/sessions` + `/api/sessions/:id/messages`：只读转录浏览器——可过滤会话列表、角色标题、可展开工具调用参数芯片、Markdown 导出、基于 `/api/sessions/:id/recap` 的网关生成回顾面板、经 `POST /api/sessions/:id/fork` 的一键会话分叉——同时修复了聊天恢复与工件浏览器的转录加载）；运行视图同时列出异步委派（`/v1/delegations`），支持按需展开汇总结果；后台监视器在运行进入等待批准状态时弹出黏性 toast 与系统通知）、petdex 宠物悬浮层（`display.pet.*` 驱动的精灵图动画画布）与孵化悬浮层（提示词 → 基础草稿挑选 → 实时孵化进度 → 领养，基于网关 `POST /api/pets/hatch` 任务 API）、系统托盘图标（Show/Quit 菜单，左键恢复窗口）、按会话模型挑选器（聊天头部模型徽章打开基于 `/api/model/options` 的弹层，经 `POST /api/sessions/:id/model` 锁定会话）、聊天内查找（Ctrl/Cmd+F 浮动查找栏，匹配高亮 + 步进）、命令面板（Ctrl/Cmd+K 模糊启动器：视图导航、会话切换与会话操作）、工件浏览器（扫描会话记录提取链接/文件/图片，按类型过滤）、学习视图（已学技能 + 记忆图谱，节点编辑/归档走 `/api/learning/*`）、双通知栈（顶部居中黏性 error/动作 toast + 右下环境确认 toast）与冷启动 CONNECTING 遮罩（P249）、首次启动引导向导（欢迎 → provider 配置指引基于 `/api/model/options` → 完成，localStorage 闩锁，设置菜单可重放；P250）、五语言（en/zh/zh-hant/ja/ar）完整界面 i18n，地球图标语言切换器支持搜索，阿拉伯语 RTL 布局（P251）、模型可见性编辑器（按 provider 精选模型集合，`-fast` 家族折叠、全部隐藏语义、featured/top-N 默认展开）过滤模型挑选器（P252）、启动失败恢复遮罩（重试/打开设置/忽略）取代冷启动失败 toast（P253）、会话导出（会话行悬停 ⭳ 或命令面板动作，经 `GET /api/sessions/:id/export` 下载 Markdown 或独立 HTML 转录）、会话挑选器（聊天输入框拦截 `/resume`/`/sessions`/`/switch` 打开的可输入过滤遮罩；P254）、空会话聊天引言（按会话种子挑选的标题/正文文案对；P255）、流式活动计时器（工具进度条实时秒数 + 逐工具实测耗时；P256）、hermes 快捷键子集（Ctrl/Cmd+Shift+M 模型挑选器、Ctrl/Cmd+N / Shift+N 新建会话、Ctrl+Tab 会话循环、Ctrl/Cmd+Shift+F 会话挑选器、Ctrl/Cmd+, 设置、Ctrl/Cmd+B 侧栏切换、输入即聚焦聊天框；P258），并管理 gateway 子进程；网关内置的本地应用 CORS 同样服务任意浏览器仪表盘

### CLI 快速开始

```bash
cargo build --release --target x86_64-unknown-linux-musl

# 交互式安装向导（provider、终端、消息平台、工具）
./ulnclaw setup                 # 或：./ulnclaw setup model|terminal|gateway|tools|agent
./ulnclaw model                 # 交互式切换 provider/模型
./ulnclaw gui                   # 启动 Tauri 桌面应用（别名：desktop）

# 生成默认配置 ~/.ulnclaw/config.toml
./ulnclaw init

# 一次性运行
./ulnclaw run "总结一下 README.md"

# 交互式聊天（斜杠命令：/new /search /memory /skills /sessions /rollback /diff /recap /goal /subgoal /focus /verbose /stash /kanban /pet /hatch /paste ……）
./ulnclaw chat
./ulnclaw chat --resume <session-id>   # 按 id 或唯一前缀恢复会话（-r）
./ulnclaw chat --continue              # 继续最近一次会话（-c）
./ulnclaw chat --continue "我的任务"   # ……或按标题/id 匹配的会话

# 管理子命令
./ulnclaw tools            # 列出工具集与已启用工具
./ulnclaw sessions list    # state.db 中的最近会话
./ulnclaw sessions search "认证重构"
./ulnclaw sessions export <session-id> --out ./exports --format md|html
./ulnclaw sessions recover ./damaged-state.db   # 离线数据库恢复
./ulnclaw sessions repair          # 修复受损 state.db 库结构（--check-only）
./ulnclaw sessions browse          # 交互式会话挑选：过滤并恢复会话（⌂ 项目徽章、右侧详情窗格、Tab 来源过滤、F2 排序切换）
./ulnclaw sessions retitle-skills  # 修复泄漏 /skill 脚手架的会话标题（--apply）
./ulnclaw sessions delete|rename|optimize # 单会话删除/重命名；FTS 合并 + VACUUM 回收空间
./ulnclaw skills list
./ulnclaw skills blueprints    # 可排程技能（skills schedule <name>）
./ulnclaw skills scan <name>   # 信任技能前的安全扫描（--json、--source、--force）
./ulnclaw cron list          # cron jobs (cron run <id> executes one immediately)
./ulnclaw suggestions        # suggested automations (accept/dismiss/catalog/clear)
./ulnclaw moa list           # MoA 预设（运行：./ulnclaw moa run "<prompt>"）
./ulnclaw models providers   # models.dev 目录（list/info/refresh）
./ulnclaw checkpoints list   # 文件系统快照（[checkpoints] enabled = true）
./ulnclaw diff               # git 工作区 diff（--staged / --all）
./ulnclaw doctor             # 诊断配置与依赖（--fix、--online、--json）
./ulnclaw insights           # 会话用量分析（--days、--source、--json）
./ulnclaw pets               # petdex 宠物：list/install/select/show/scale/doctor/hatch
./ulnclaw status             # 全组件状态总览（--deep）
./ulnclaw logs               # 查看/过滤日志（-f、-n、--level、--since、--component）
./ulnclaw update --check   # 检查更新（ulnclaw update 应用：stash -> ff 拉取 -> 重建）
./ulnclaw config           # 配置 show/get/set/unset（env 风格键写入 .env）
./ulnclaw secrets status   # 外部秘密源（secrets sync [--apply] 立即拉取）
./ulnclaw secrets bitwarden setup   # 向导：安装 bws、存令牌、选项目（另有 install/status/token/disable；onepassword setup/status/set/remove/disable）
./ulnclaw computer-use status # cua-driver 后台桌面控制（doctor/install）
./ulnclaw plugins list      # 插件与 shell 钩子（enable/disable/accept-hooks）
./ulnclaw kanban list       # kanban 任务引擎（init/boards/create [--max-runtime 30s|5m|2h|1d --max-retries N --workspace KIND --branch B --goal --initial-status blocked --model M --provider P --project P]/claim/done/review/block/comment/swarm/specify/decompose/diagnostics/schedule/promote/reclaim/reassign/edit/set-model [--provider P]/attach/tail/log/runs/context/repair/assignees/notify-subscribe/stats/...）
./ulnclaw project list     # 一等项目登记簿（create/show/add-folder/remove-folder/rename/set-primary/use/archive/restore/bind-board；kanban create --project 锚定 worktree）
./ulnclaw project scan     # git 仓库发现缓存（scan [--root P --max-depth N]/repos [--clear]）
./ulnclaw hooks doctor      # 逐个探测已同意的钩子（list/test/revoke）
./ulnclaw pairing list      # 陌生发送者的 DM 配对码（approve/revoke/clear-pending）
./ulnclaw weixin login      # 微信 iLink 扫码登录（[messaging.weixin]）
./ulnclaw auth login        # OAuth 设备流登录（status/refresh/logout/open）
./ulnclaw sync status       # 技能同步（pull/push/now/enable/disable/device）
./ulnclaw completion bash  # shell 补全脚本（bash/zsh/fish/elvish/powershell）
./ulnclaw dump             # 可粘贴的装机摘要，用于求助排查（--show-keys）
./ulnclaw version          # 版本 + 安装信息 + 升级状态
./ulnclaw uninstall        # 移除代码/PATH 条目/包装脚本（--full 连数据清除、--dry-run、--yes）
./ulnclaw memory           # 持久记忆状态；`memory reset [all|memory|user]` 清除
./ulnclaw approvals        # 终端审批模式；`approvals manual|smart|off` 设置
./ulnclaw prompt-size      # 系统提示词 + 工具 schema 体积（--json）
./ulnclaw debug report     # 脱敏诊断包，用于求助分享（--no-redact）
./ulnclaw bundles          # 技能束：一个 /命令 加载一组技能
./ulnclaw import-agent     # 导入 Claude Code / Codex 配置（--dry-run）
./ulnclaw security audit   # 固定版本 MCP 包的 OSV.dev 审计（--json）
./ulnclaw fallback         # 回退链管理（add/remove/clear provider:model 条目）
./ulnclaw backup           # home 目录 zip 备份（-q 快速快照，backup list/restore/prune）
./ulnclaw import b.zip     # 恢复备份 zip（跳过运行时状态文件，机密文件 0600）

# 浏览器自动化：auto 模式自动启动托管的无头 Chrome/Chromium；
# 也可将 browser_* 工具指向已开启远程调试的浏览器
export ULNCLAW_BROWSER_CDP=http://127.0.0.1:9222     # 或 ws://.../devtools/browser/... 或 "auto"
# 也可让 browser_* 按需创建云浏览器会话（hermes 云 provider）：
#   config.toml [browser] cloud_provider = "browserbase" | "browser-use" | "firecrawl" | "local"
#   + 凭据：BROWSERBASE_API_KEY+BROWSERBASE_PROJECT_ID / BROWSER_USE_API_KEY / FIRECRAWL_API_KEY

# HTTP 网关（OpenAI 兼容 API 服务器，默认 127.0.0.1:8642）
./ulnclaw gateway --host 127.0.0.1 --port 8642
# 消息平台随网关运行
# （[messaging.telegram|discord|slack|signal|weixin|qq|yuanbao|email|mattermost|matrix|dingtalk|wecom|homeassistant|whatsapp|irc|ntfy|simplex|buzz|photon]，
#  另有 webhook 平台：whatsapp_cloud/msgraph/webhook/bluebubbles/feishu/sms/teams/line/google_chat/raft/a2a）
# Tauri 桌面应用（desktop/）与任意浏览器仪表盘经 HTTP/SSE 连接本网关
# （内置本地应用 CORS，见 desktop/README.md）
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
- 设计文档：[多路复用网关](docs/design/multiplexing-gateway.md)（多 profile 路由 + fail-closed 密钥作用域）· [浏览器 CDP 客户端](docs/design/browser-cdp.md)（CDP 会话层 + `/v1/browser/*` 控制面）

### 构建与测试

```bash
cargo test                     # 990 个测试
cargo build --release --target x86_64-unknown-linux-musl   # 静态二进制
```

### 许可证

MIT OR Apache-2.0
