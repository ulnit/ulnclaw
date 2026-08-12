# Desktop React Shell Migration (v0.4 → v0.5)

> **Audience:** contributors working on the desktop webview frontend
> **Source files:** `desktop/src/boot.ts` (shell selector), `desktop/src/react/*` (React tree), `desktop/src/hermes/*` (verbatim hermes primitives), `desktop/src/i18n.ts` (locale catalogs), `desktop/src/gateway.ts` (framework-agnostic GatewayClient)
> **Related:** `docs/en/hermes-parity.md` (desktop rows), `README.md` (Desktop App sections)
> **Hermes parity:** `apps/desktop/` frontend stack (v2026.8.3) — React + tailwind token architecture, cmdk palette, i18n context, hermes button/loader primitives

## Overview

Through v0.3.x the desktop webview ran a hand-written vanilla-TS shell
(`desktop/src/main.ts` + `style.css`). It reached full feature parity but
its CSS grew organically and its layout/typography drifted from the
hermes desktop reference. v0.4.0 introduced a second, React-based shell
built directly on hermes' frontend primitives; v0.5.0 flipped it to be
the default; the classic shell remains reachable behind
`?shell=classic` (or `localStorage["ulnclaw.shell"] = "vanilla"`) for one
more release before retirement.

## Boot selection

`index.html` loads only `src/boot.ts`. It reads, in priority order:

1. `?shell=` URL param (`react` | `classic` | `vanilla`),
2. persisted `ulnclaw.shell` choice,
3. default `react` (since v0.5.0).

and dynamic-imports either `./react/main` or `./main`, so each shell is
a separate bundle chunk and the unused one never parses.

## Architecture

- **GatewayClient (`src/gateway.ts`)** is framework-agnostic: every
  endpoint (sessions, chat SSE, jobs, usage, models, skills, kanban,
  projects, webhooks, runs, plugins, pairing, profiles, config, doctor,
  logs, dashboard themes/fonts, retitle) is a plain async method. Both
  shells consume it; the React tree never touches `fetch` directly.
- **Hermes primitives (`src/hermes/`)** — `button.tsx`, `loader.tsx`,
  `utils.ts`, `tokens.css` — are verbatim ports of the hermes desktop
  components. `tokens.css` bridges the gateway dashboard theme seeds
  (`--seed-*`) into the `--dt-*`/`--ui-*` custom properties the tailwind
  `@theme inline` map consumes, so all eight hermes palettes and the
  font catalog work unchanged in the React shell.
- **Views (`src/react/views/`)** — sixteen routed views: Chat, Sessions,
  Jobs, Usage, Models, Skills, Kanban, Projects, Runs, Webhooks,
  Plugins, Pairing, Profiles (read-only), Config, Doctor, Settings.
  `Sidebar.tsx` exports the `ShellView` union and renders the nav in
  three captioned groups (primary / Workspace / System); `App.tsx`
  routes; `Palette.tsx` (cmdk, Ctrl/Cmd+K) lists every view plus
  session actions.
- **i18n (`src/i18n.ts`)** — five locales (en, zh, zh-hant, ja, ar) with
  the full hermes-derived catalog. The React shell subscribes through
  `useT()` (`useSyncExternalStore` over `onLocaleChange`); the locale
  persists in `localStorage["ulnclaw.locale"]` and the picker lives in
  Settings › Language. `applyStatic()` still translates the vanilla
  shell's `data-i18n` chrome.

## Chat view contract

`ChatView.tsx` renders transcripts with `marked` + `DOMPurify`
(`.md-body` typography in `shell.css`), tool cards as collapsible
scaffold rows driven by the SSE `tool_card` events, day dividers,
image attachments (file → dataURL → `chatStream(..., images)`), the
lemniscate busy loader, a model picker dropdown over
`GET /api/model/options` + `POST /api/model/set`, and a slash-command
completion popup (34 hermes slash commands, i18n descriptions,
Arrow/Tab/Enter/Escape keys) whose selection is sent through the normal
submit path so the gateway agent loop executes it.

## Parity gaps vs the classic shell (accepted for v0.5.x)

- no TTS / completion chime in the React shell;
- no per-session popout window or file-tree side panel;
- gateway process management (spawn/respawn, tray, autostart, quit
  guard, gateway log viewer) lives in the Rust/Tauri layer and is
  therefore identical for both shells; a gateway outage surfaces as a
  retry banner (`boot.unreachable`) instead of the classic boot card
  with the Diagnostics expander.

These are tracked for a follow-up release; none blocks the classic
shell's retirement.

## Retirement plan (v0.6.0)

1. Port the missing chrome (TTS, file tree, diagnostics expander) or
   explicitly drop them;
2. delete `src/main.ts`, `style.css` and vanilla-only modules
   (keep `i18n.ts`, `gateway.ts`, `icons.ts`);
3. slim `index.html` to the React entry and drop the boot selector.
