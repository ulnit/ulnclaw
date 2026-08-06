# Multiplexing Gateway — Multi-Profile Routing & Fail-Closed Secret Scopes

> **Audience:** gateway operators and contributors
> **Source files:** `src/gateway/mod.rs` (`ProfileHub`, `profile_dispatch`, `serve_multiplex`), `src/secret_scope.rs`, `src/main.rs` (gateway startup)
> **Related:** `src/secrets.rs` (external secret sources), `docs/en/hermes-parity.md` (HTTP gateway row)
> **Hermes parity:** `agent/secret_scope.py`, `gateway/platforms/api_server.py` profile-prefix middleware (v2026.8.3). This is the document hermes' `secret_scope.py` refers to as "Workstream A".

## Overview

By default one gateway process serves one profile: one home directory, one
`config.toml`, one `.env`, one agent. **Profile multiplexing** lets a single
gateway process serve **many isolated profiles** at once. Each profile gets:

- its own home: `<home>/profiles/<name>/` (sessions, memory, skills, state.db);
- its own config overlay: the `[profiles.<name>]` table merged over the base config;
- its own agent/store stack, built lazily on first request and cached;
- its own credentials: `<home>/profiles/<name>/.env` plus hydrated external
  secret sources — **never** merged into the process environment.

Enable it with:

```toml
[gateway]
multiplex_profiles = true

[profiles.work]
# overrides for the "work" profile (model, persona, tools, ...)

[profiles.personal]
# ...
```

Requests reach a profile through the mirrored path prefix
`/p/<profile>/<path>` — e.g. `POST /p/work/v1/chat/completions`. Every native
route exists under the mirror with identical behavior and identical bearer
auth.

## Threat model: why not union the `.env` files?

The naive implementation loads every profile's `.env` into the process
environment at startup. That is wrong on two axes:

1. **Cross-profile leakage.** Profile A's provider key and profile B's
   platform token would coexist in one flat namespace; whichever profile is
   loaded last wins. A turn for profile B could silently authenticate with
   profile A's key — billing, quota, and identity all cross the wire.
2. **Subprocess inheritance.** Every tool subprocess (`terminal`, MCP
   servers, plugin tools) inherits the process environment. Unioning all
   profiles' secrets into `env` hands *every* profile's credentials to
   *every* spawned process, including processes that untrusted tool output
   can influence.

The multiplexing gateway therefore never mutates the process environment with
profile secrets. Credentials live in **per-request, task-local scopes**.

## Routing layer (`ProfileHub`)

`serve_multiplex` registers one extra route covering all HTTP methods:

```
/p/:profile/*rest  →  profile_dispatch
```

Resolution policy (hermes `_resolve_request_profile` parity):

| `[gateway] multiplex_profiles` | profile in `[profiles]` | result |
|---|---|---|
| off | any | prefix **ignored** — the default profile serves the request (a valid-looking route never 404s just because multiplexing is disabled) |
| on | yes | per-profile router (lazy build, cached in `ProfileHub.cache`) |
| on | no | `404 Unknown or unconfigured profile` |

`profile_dispatch` then:

1. rewrites the URI — strips `/p/<profile>` and keeps the query string;
2. builds the profile's secret scope via the `ProfileScopeBuilder`
   (`build_profile_secret_scope(<home>/profiles/<name>)`);
3. re-dispatches the request to the profile's cached router through
   `tower::ServiceExt::oneshot`, wrapped in `scope_secrets(scope, ...)` so
   every credential read inside the request resolves against that profile.

Routers are built by an async factory (`ProfileRouterBuilder`) that runs
`build_gateway_stack` for the profile — full agent + session store +
platform senders — and are memoized for the process lifetime.

## Secret scope layer (`src/secret_scope.rs`)

### The task-local scope

```rust
tokio::task_local! {
    static SECRET_SCOPE: Arc<HashMap<String, String>>;
}
```

- `scope_secrets(mapping, future)` installs the mapping around a future — the
  Rust analogue of hermes' `contextvar` + `set_secret_scope`.
- `spawn_scoped(future)` spawns a task that inherits the current scope.
  **Important:** tokio task-locals do *not* propagate into
  `tokio::spawn`-ed tasks automatically (Python's `copy_context()` does
  propagate through the hermes equivalent). Any spawn site that crosses a
  scope boundary — per-turn run spawners, adapter loops, tracked runs — must
  use `spawn_scoped`, or the scope is lost inside the spawned task.

### Fail-closed resolution: `get_secret`

Resolution order:

1. **Genuinely-global vars** (`is_global_env`) always read the process
   environment — deployment settings, not profile secrets.
2. **Scope installed:** read the scope. Under multiplexing the scope is
   *authoritative* — a missing key returns the default and does **not** fall
   through to the process environment (which may hold another profile's
   value). With multiplexing **off**, a scope miss falls through: the scope
   is an overlay, not a blindfold — single-profile deployments legitimately
   inject credentials via systemd `Environment=`, secret-manager wrappers,
   or shell exports, and cron jobs install a scope around every run.
3. **No scope installed:**
   - multiplex **inactive** (default): read the process environment —
     byte-for-byte the legacy behavior;
   - multiplex **active**: **FAIL CLOSED** with `UnscopedSecretError`. A
     credential read that reaches `get_secret` outside any profile scope is
     a bug; failing loud at that exact line beats leaking whichever value
     happens to be in the process environment.

`get_secret_lenient` is the escape hatch for call sites outside the
multiplexed gateway where fail-closed would only produce noise.

### The global-env allowlist

Some variables are process/deployment-level by nature and must keep reading
the process environment even in multiplex mode — routing them through the
fail-closed path would wrongly crash the gateway. Membership is exact name
or prefix:

- exact: `ULNCLAW_HOME`, `ULNCLAW_PROFILE`, `PATH`, `HOME`, `TZ`, kanban
  paths (`ULNCLAW_KANBAN_DB`, ...), api-server *listener* settings
  (`API_SERVER_ENABLED/HOST/PORT/CORS_ORIGINS`), and a short list of runtime
  tuning knobs;
- prefixes: `ULNCLAW_KANBAN_`, `ULNCLAW_TELEGRAM_` (tuning knobs — **not**
  the bot token), `TERMINAL_`.

Policy: **keep this list tight.** When in doubt, a value is a profile secret,
not a global. Gateway auth keys (`ULNCLAW_GATEWAY_KEY`, `API_SERVER_KEY`)
are deliberately *not* global — they stay profile-scoped.

### Scope construction: `build_profile_secret_scope`

```
<profile-home>/.env  ──parse (no env mutation)──┐
                                                ├─→ scope mapping
external sources (secrets vaults) ──hydrate─────┘
```

- `load_env_scoped` parses the `.env` KEY=VALUE subset (`export` prefix,
  full-line and inline comments with python-dotenv 1.2.2 semantics, quoted
  values with `\"`/`\\` escapes reversed, UTF-8 BOM stripped) into a plain
  map **without touching the process environment** — that isolation is the
  whole point.
- `hydrate_profile_secret_sources` resolves the profile's configured
  external secret sources once per home (per-home snapshot registry,
  canonicalized path keys) and records the contributed values. Fail-open:
  any source error degrades to an empty contribution — external sources must
  never block routing. This covers the first-turn-to-a-secondary-profile
  case where the process-global startup path never ran for that profile.
- Genuinely-global names are retained out of the final mapping — they are
  read from the environment directly by `get_secret`.

### Scope installation points

| Where | What is installed |
|---|---|
| `profile_dispatch` | the request's profile scope around the whole dispatched request |
| cron scheduler | the gateway home's scope around every job run |
| `spawn_tracked_run` / per-turn spawners | inherit via `spawn_scoped` |
| `serve_multiplex` startup | `set_multiplex_active(true)` — arms the fail-closed path |

## Failure modes & debugging

- **`UnscopedSecretError` in gateway logs** — a credential read escaped the
  profile scope. The fix is to wrap the call path in `scope_secrets(...)` /
  switch the spawn site to `spawn_scoped`; it is **not** to widen the
  global-env allowlist.
- **404 on `/p/<name>/...`** with multiplexing on — the profile is not
  declared under `[profiles]`.
- **Scope miss returning defaults** — expected under multiplexing: the scope
  is authoritative. Put the credential in that profile's `.env` (or a
  configured external source), not in the gateway process environment.

## Testing notes

Tests that toggle the multiplex flag must hold
`crate::secret_scope::test_multiplex_lock()` (poison-tolerant) — the flag is
process-global. Env-sensitive tests likewise serialize on
`crate::models_dev::test_env_lock()`.
