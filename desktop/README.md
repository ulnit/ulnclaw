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
