# Desktop Shell Design — `desktop-electron/` (ulnclaw desktop)

> **Audience:** contributors working on the desktop app or its gateway surface
> **Source of truth:** `desktop-electron/UPSTREAM-SYNC.md` (sync policy + divergence catalog)
> **Related:** `docs/en/hermes-parity.md` / `docs/zh/hermes-parity.md` (Desktop GUI rows), `README.md` (Desktop App sections)

## Status (v0.7.0+)

The desktop is **ulnclaw desktop**: a vendored, byte-faithful copy of the
hermes Electron desktop (`NousResearch/hermes-agent` `apps/desktop` +
`apps/shared`, MIT; vendored revision v2026.8.3), replacing the earlier
in-house Tauri 2 shell (`desktop/`, retired and deleted).

Policy, per maintainer direction:

- The desktop code **directly tracks hermes desktop**. Upstream upgrades are
  pulled in mechanically by `desktop-electron/scripts/sync-from-hermes.mjs`
  (brand rename + divergence replay, verified byte-for-byte in check mode).
- The **only intentional difference** is the backend gateway: ulnclaw desktop
  spawns and supervises the ulnclaw Rust gateway; hermes desktop does the
  same against the hermes Python backend.
- If the shell needs a capability the gateway lacks, implement it in the
  gateway (`src/gateway/`, WS JSON-RPC in `src/gateway_ws.rs`) — never by
  forking the vendored shell.

## Architecture

```
┌─────────────────────────── ulnclaw desktop ───────────────────────────┐
│  Electron main process (electron/main.ts, vendored + 1 divergence)    │
│   • resolves gateway binary: resources/binaries/ulnclaw(.exe) → PATH  │
│   • spawn `ulnclaw gateway --host 127.0.0.1 --port 0` (ULNCLAW_DESKTOP=1)
│   • readiness: "listening on http://…:PORT" (backend-ready.ts)        │
│   • health probes + capped respawn, log → ~/.ulnclaw/gateway.log      │
│   • tray, native menus, ulnclaw:// deep links, single-instance,       │
│     window-state persistence, boot diagnostics                        │
├────────────────────────────────────────────────────────────────────────┤
│  React 19 + Vite renderer (src/, vendored verbatim)                   │
│   • sixteen views (chat, sessions, jobs, usage, models, skills,       │
│     kanban, projects, runs, webhooks, plugins, pairing, profiles,     │
│     config, doctor, settings) + command palette + themes + i18n       │
│   • xterm.js terminal panes, right-hand file tree + git review        │
├────────────────────────────────────────────────────────────────────────┤
│  Wire to gateway (identical to hermes desktop ↔ hermes gateway)       │
│   • HTTP/SSE: /api/*, /v1/* (OpenAI-compatible), /api/desktop/events  │
│   • WebSocket JSON-RPC: prompt.submit streaming (message/tool events),│
│     session control, approvals, model options (shared/json-rpc-gateway│
│     .ts, vendored verbatim)                                           │
└────────────────────────────────────────────────────────────────────────┘
```

## Divergence surface (7 files, ~70 lines)

Cataloged with exact patches in `desktop-electron/UPSTREAM-SYNC.md`:
gateway argv (`serve`→`gateway`), readiness regex, bundled-binary
resolution in `main.ts`, packaging fields in `package.json`, vendored-layout
paths in `vite.config.ts`/`tsconfig.json`/`scripts/assert-root-install.mjs`.

## Packaging & distribution

electron-builder (config lives in the vendored `package.json` + the
packaging divergence): Windows NSIS (`ulnclaw-<ver>-win-x64.exe`), macOS dmg
(arm64/x64), Linux AppImage/deb/rpm; artifact naming
`ulnclaw-${version}-${os}-${arch}.${ext}`; the statically linked gateway
binary is staged into `resources/binaries/` and shipped via `extraResources`.
CI: `.github/workflows/release-desktop.yml` on `v*` tags.

## Historical note

v0.4–v0.6 shipped a Tauri 2 shell in `desktop/` (vanilla shell → React
hermes-parity shell). v0.7.0 deleted it in favor of the vendored hermes
desktop: identical UX to the upstream reference at zero maintenance drift,
with the gateway as the sole integration seam.
