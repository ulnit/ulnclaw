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
  The chat-header model badge opens a per-session model picker overlay
  (`GET /api/model/options` → `POST /api/sessions/:id/model` lock; the
  gateway-default row resets the lock) — hermes model-picker parity.
  Ctrl/Cmd+F opens a find-in-chat bar (DOM highlighting, n/m counter,
  Enter/Shift+Enter stepping, Esc clears) — hermes find-bar parity.
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
- **Jobs** (tab in the sidebar) — cron/scheduled-job dashboard over
  `/api/jobs` (same scheduler as the `ulnclaw cron` CLI): job rows with a
  status dot, schedule, skills, prompt preview and next-run countdown;
  pause/resume, run-now, inline edit (prompt + schedule), delete, and a
  create dialog (name/schedule/prompt/skills/repeat), 10 s polling.
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
- **System tray** — tray icon with a Show/Quit menu; left-click restores
  the main window. Creation is best-effort: without a status-notifier
  implementation the app continues windowed.

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

### Windows install (GUI)

Prerequisites:

- **Rust** via [rustup](https://rustup.rs) — the default MSVC toolchain
  (install the Visual Studio Build Tools "Desktop development with C++"
  workload if you don't have a linker yet).
- **Node.js ≥ 20** + npm.
- **WebView2 runtime** — preinstalled on Windows 11 and current
  Windows 10; otherwise install Microsoft's Evergreen WebView2.

Steps (PowerShell):

```powershell
git clone https://gitee.com/ushaw/ulnclaw.git
cd ulnclaw

# 1. Core binary (CLI + gateway) — plain MSVC target, no musl
cargo build --release
copy target\release\ulnclaw.exe $env:USERPROFILE\.cargo\bin\

# 2. First-run onboarding
ulnclaw setup        # wizard: provider key, terminal, tools
ulnclaw init         # optional: write default config

# 3. Desktop shell (NSIS installer)
cd desktop
npm install
npm run tauri build -- --bundles nsis
# installer: src-tauri\target\release\bundle\nsis\ulnclaw_0.2.0_x64-setup.exe
```

The shell locates `ulnclaw.exe` on PATH (or in `.local/bin`, `bin`,
`.cargo/bin` under `%USERPROFILE%`), spawns `ulnclaw gateway --port
8642` and hosts the chat UI — launch the installed app and use the GUI.

Notes:

- `--bundles nsis` skips the MSI target (needs the WiX toolset); plain
  `npm run tauri build` builds every bundle.
- On Windows the gateway stop is best-effort at app exit (no SIGTERM),
  and the pidfile single-instance guard is disabled (no `/proc`).
- The Bitwarden `bws` asset path under secrets is untested on Windows.

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
  聊天头部模型徽章打开按会话模型挑选弹层（`GET /api/model/options` →
  `POST /api/sessions/:id/model` 锁定；网关默认行重置锁定）—— hermes
  model-picker 对位。
  Ctrl/Cmd+F 打开聊天内查找栏（DOM 高亮、n/m 计数、Enter/Shift+Enter 步进、
  Esc 清除）—— hermes find-bar 对位。
- **看板**（侧栏标签页）—— 基于 `/api/kanban/*` 的四列卡片墙（与 CLI、
  agent 工具共用同一引擎和 `kanban.db`）：快速添加、完成/阻塞/解除阻塞、
  带评论的任务详情、看板切换、5 秒轮询。
- **项目**（侧栏标签页）—— 基于 `/api/projects/*` 的一等多文件夹项目
  登记簿（与 `ulnclaw project` CLI 共用同一 `projects.db`）：项目卡片
  支持创建/启用/归档/删除、文件夹管理与主目录切换、board 绑定、文件
  系统仓库扫描发现缓存与一键收养。
- **任务**（侧栏标签页）—— 基于 `/api/jobs` 的定时任务仪表盘（与
  `ulnclaw cron` CLI 共用同一调度器）：任务行带状态点、调度表达式、
  技能、提示词预览与下次运行倒计时；支持暂停/恢复、立即运行、行内
  编辑（提示词 + 调度）、删除与创建对话框（名称/调度/提示词/技能/
  重复），10 秒轮询。
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
- **系统托盘** —— 带 Show/Quit 菜单的托盘图标，左键恢复主窗口；
  创建为尽力而为：缺少状态通知器实现时应用继续以纯窗口模式运行。

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

### Windows 安装（图形界面）

前置条件：

- **Rust**：经 [rustup](https://rustup.rs) 安装，默认 MSVC 工具链
  （若还没有链接器，安装 Visual Studio Build Tools 的
  “使用 C++ 的桌面开发”工作负载）。
- **Node.js ≥ 20** + npm。
- **WebView2 运行时**：Windows 11 与较新的 Windows 10 已内置；
  否则安装微软 Evergreen WebView2。

步骤（PowerShell）：

```powershell
git clone https://gitee.com/ushaw/ulnclaw.git
cd ulnclaw

# 1. 核心二进制（CLI + 网关）——直接 MSVC target，不用 musl
cargo build --release
copy target\release\ulnclaw.exe $env:USERPROFILE\.cargo\bin\

# 2. 首次配置
ulnclaw setup        # 向导：provider 密钥、终端、工具
ulnclaw init         # 可选：写入默认配置

# 3. 桌面外壳（NSIS 安装包）
cd desktop
npm install
npm run tauri build -- --bundles nsis
# 安装包：src-tauri\target\release\bundle\nsis\ulnclaw_0.2.0_x64-setup.exe
```

外壳会从 PATH（或 `%USERPROFILE%` 下的 `.local/bin`、`bin`、
`.cargo/bin`）定位 `ulnclaw.exe`，拉起 `ulnclaw gateway --port 8642`
并承载聊天界面——装好后直接启动应用即可使用图形界面。

说明：

- `--bundles nsis` 跳过 MSI（需要 WiX 工具集）；不带参数执行
  `npm run tauri build` 会构建全部安装包。
- Windows 上退出应用时网关进程为尽力而为的停止（无 SIGTERM），
  单实例 pidfile 守卫不生效（无 `/proc`）。
- secrets 的 Bitwarden `bws` 资产路径在 Windows 上未经测试。
