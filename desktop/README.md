# ulnclaw desktop 🖥️

[English](#english) | [中文](#中文)

---

## English

A [Tauri](https://tauri.app) desktop shell for the ulnclaw agent: a native
window hosting the chat UI, with the Rust side owning the `ulnclaw gateway`
child process.

> hermes-agent ships an Electron desktop app (`apps/desktop`). ulnclaw
> deliberately uses Tauri instead: smaller binaries, native webview, and the
> gateway process stays a plain child of the Rust shell.

### Architecture

```
┌───────────────────────────── Tauri window ─────────────────────────────┐
│  webview (Vite/TS bundle)                                              │
│    index.html + src/main.ts + src/gateway.ts                           │
│        │ HTTP/SSE (127.0.0.1:<port>)      │ IPC (@tauri-apps/api)      │
│        ▼                                  ▼                            │
│  ulnclaw gateway ◄── spawned/stopped by ── src-tauri (lib.rs)          │
│  (agent engine, sessions, approvals)      find_ulnclaw_binary          │
│                                           default_gateway_port         │
│                                           spawn_gateway / stop_gateway │
└────────────────────────────────────────────────────────────────────────┘
```

- The **webview talks HTTP directly to the gateway** (`/api/sessions`,
  `/api/chat` SSE streaming, `/api/config`, ...). The same endpoints power
  the REPL gateway mode and any browser dashboard, so the desktop UI is a
  thin client with no bespoke bridge protocol. Session rows show hover
  rename ✎ / delete 🗑 actions (`PATCH`/`DELETE /api/sessions/:id`), and
  streaming turns render a live `⚙ <tool> — <status>` strip fed by the
  named `hermes.tool.progress` SSE events.
- The **Tauri side only manages the process lifecycle**: locate the
  `ulnclaw` binary (PATH → `~/.local/bin` → `~/bin` → `~/.cargo/bin`),
  read the default port from `~/.ulnclaw/config.toml` (`[gateway] port`,
  default `8642`), spawn `ulnclaw gateway --port <N>` on demand, and send
  SIGTERM on exit.
- Gateway CORS: `serve_multiplex` adds a permissive local-app CORS layer
  (origin echo + OPTIONS preflight), which is what lets the webview — and
  any local browser dashboard — call the API. The gateway still binds
  `127.0.0.1` and enforces the API key when one is configured.

### Views and widgets

- **Chat** — the session browser + streaming chat surface.
- **Kanban** (tab in the sidebar) — four-column card wall over
  `/api/kanban/*` (same engine + `kanban.db` as the CLI and agent tools):
  quick-add, complete/block/unblock actions, task detail with comments,
  board switcher, 5 s polling.
- **Projects** (tab in the sidebar) — first-class multi-folder project
  registry over `/api/projects/*` (same `projects.db` as the `ulnclaw
  project` CLI): project cards with create/use/archive/delete, folder
  management with primary switching, board binding, a filesystem repo
  scan feeding the discovery cache, and one-click adoption of discovered
  repos into projects.
- **Pet overlay** — the petdex mascot as an animated spritesheet canvas in
  the bottom-right corner: `display.pet.*` config via `/api/pets/config`,
  sheet from `/api/pets/:slug/spritesheet`, working (`running` row) while
  `/v1/runs` has live runs, idle otherwise, click to wave.
- **Hatch overlay** (🥚 button in the sidebar footer) — hatch a brand-new
  pet from the GUI (hermes pet-generate parity): prompt + style + draft
  count → base-draft grid → pick one → live row-generation progress →
  spritesheet preview, auto-adopted on success. Rides the gateway hatch-job
  API (`POST /api/pets/hatch`, `GET /api/pets/hatch/:id`,
  `POST /api/pets/hatch/:id/pick|cancel`,
  `GET /api/pets/hatch/:id/draft/:index`).

### Prerequisites

- **Rust** (stable) with the host target.
- **Node.js ≥ 20** + npm.
- **Linux**: `webkit2gtk-4.1` and `libsoup3` dev packages (Tauri v2), e.g.
  on Debian/Ubuntu:

  ```sh
  sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev \
      libayatana-appindicator3-dev librsvg2-dev
  ```

- A built `ulnclaw` binary on PATH (or in `~/.local/bin`, `~/bin`,
  `~/.cargo/bin`):

  ```sh
  cargo build --release --target x86_64-unknown-linux-musl
  ```

### Develop

```sh
cd desktop
npm install
npm run tauri dev      # vite dev server on :5180 + tauri window
```

The UI falls back to plain browser mode when the Tauri IPC bridge is
absent, so `npm run dev` alone is enough to iterate on the interface
against a manually started `ulnclaw gateway`.

### Build an app bundle

```sh
cd desktop
npm run tauri build    # bundle under src-tauri/target/release/bundle/
```

---

## 中文

ulnclaw 的 [Tauri](https://tauri.app) 桌面外壳：原生窗口承载聊天界面，
Rust 侧负责托管 `ulnclaw gateway` 子进程。

> hermes-agent 自带 Electron 桌面应用（`apps/desktop`）。ulnclaw 刻意选用
> Tauri：二进制更小、使用系统原生 webview，gateway 进程保持为 Rust 外壳的
> 普通子进程。

### 架构

- **webview 直接通过 HTTP 与 gateway 通信**（`/api/sessions`、
  `/api/chat` SSE 流式、`/api/config` 等）。这些端点与 REPL gateway 模式、
  浏览器仪表盘完全共用，桌面 UI 只是一个瘦客户端，没有额外的桥接协议。
  会话行悬停出现重命名 ✎ / 删除 🗑 操作（`PATCH`/`DELETE
  /api/sessions/:id`），流式回合通过命名 SSE 事件 `hermes.tool.progress`
  实时渲染 `⚙ <tool> — <status>` 工具进度条。
- **Tauri 侧只管理进程生命周期**：定位 `ulnclaw` 二进制（PATH →
  `~/.local/bin` → `~/bin` → `~/.cargo/bin`），从
  `~/.ulnclaw/config.toml` 读取默认端口（`[gateway] port`，默认
  `8642`），按需启动 `ulnclaw gateway --port <N>`，退出时发送 SIGTERM。
- gateway CORS：`serve_multiplex` 增加了面向本地应用的宽松 CORS 层
  （回显 Origin + OPTIONS 预检），webview 与任何本地浏览器仪表盘因此可以
  直接调用 API。gateway 依然绑定 `127.0.0.1`，配置了 API key 时依旧强制校验。

### 视图与挂件

- **聊天** —— 会话浏览 + 流式聊天主界面。
- **看板**（侧栏标签页）—— 基于 `/api/kanban/*` 的四列卡片墙（与 CLI、
  agent 工具共用同一引擎和 `kanban.db`）：快速添加、完成/阻塞/解除阻塞、
  带评论的任务详情、看板切换、5 秒轮询。
- **宠物悬浮层** —— 右下角的 petdex 吉祥物精灵图动画画布：
  `/api/pets/config` 读取 `display.pet.*` 配置，
  `/api/pets/:slug/spritesheet` 加载精灵图，`/v1/runs` 有运行中任务时
  播放工作状态，否则空闲，点击挥手。
- **孵化悬浮层**（侧栏底部 🥚 按钮）—— 从 GUI 孵化全新宠物（hermes
  pet-generate 对位）：提示词 + 风格 + 草稿数 → 基础草稿网格 → 挑选 →
  实时行生成进度 → 精灵图预览，成功后自动领养。基于网关孵化任务 API
  （`POST /api/pets/hatch`、`GET /api/pets/hatch/:id`、
  `POST /api/pets/hatch/:id/pick|cancel`、
  `GET /api/pets/hatch/:id/draft/:index`）。

### 前置条件

- **Rust**（stable）及宿主 target。
- **Node.js ≥ 20** + npm。
- **Linux**：`webkit2gtk-4.1` 与 `libsoup3` 开发包（Tauri v2），Debian/Ubuntu：

  ```sh
  sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev \
      libayatana-appindicator3-dev librsvg2-dev
  ```

- 已构建的 `ulnclaw` 二进制，位于 PATH（或 `~/.local/bin`、`~/bin`、
  `~/.cargo/bin`）：

  ```sh
  cargo build --release --target x86_64-unknown-linux-musl
  ```

### 开发

```sh
cd desktop
npm install
npm run tauri dev      # vite 开发服务器 :5180 + tauri 窗口
```

没有 Tauri IPC 桥时 UI 自动退回纯浏览器模式，因此单独 `npm run dev`
配合手动启动的 `ulnclaw gateway` 即可迭代界面。

### 打包

```sh
cd desktop
npm run tauri build    # 产物位于 src-tauri/target/release/bundle/
```
