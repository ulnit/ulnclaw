// ulnclaw desktop UI — session browser + streaming chat against the
// ulnclaw gateway. Tauri commands manage the gateway child process;
// everything else is plain HTTP (gateway.ts).

import { GatewayClient, loadSettings, saveSettings } from "./gateway";
import type { GatewaySettings, SessionRow, SkillRow, ToolCardEvent } from "./gateway";

// Tauri IPC is optional: the same UI runs in a plain browser tab against
// a gateway (dev mode), so guard the dynamic import.
interface DesktopBridge {
  findBinary(): Promise<string | null>;
  spawnGateway(binary: string, port: number): Promise<number>;
  stopGateway(pid: number): Promise<void>;
  defaultPort(): Promise<number>;
}

async function loadBridge(): Promise<DesktopBridge | null> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return {
      findBinary: () => invoke<string | null>("find_ulnclaw_binary"),
      spawnGateway: (binary, port) => invoke<number>("spawn_gateway", { binary, port }),
      stopGateway: (pid) => invoke<void>("stop_gateway", { pid }),
      defaultPort: () => invoke<number>("default_gateway_port"),
    };
  } catch {
    return null; // plain browser — no process management
  }
}

const state = {
  settings: loadSettings(),
  client: null as GatewayClient | null,
  bridge: null as DesktopBridge | null,
  sessions: [] as SessionRow[],
  current: null as SessionRow | null,
  busy: false,
  managedPid: null as number | null,
  skills: [] as SkillRow[],
  pendingUploads: [] as { path: string; mime: string; bytes: number }[],
};

const el = {
  dot: document.getElementById("gateway-dot")!,
  sessionList: document.getElementById("session-list")!,
  newSession: document.getElementById("new-session") as HTMLButtonElement,
  messages: document.getElementById("messages")!,
  chatTitle: document.getElementById("chat-title")!,
  modelBadge: document.getElementById("model-badge")!,
  input: document.getElementById("input") as HTMLTextAreaElement,
  send: document.getElementById("send") as HTMLButtonElement,
  toolProgress: document.getElementById("tool-progress")!,
  slashPop: document.getElementById("slash-pop")!,
  attachChips: document.getElementById("attach-chips")!,
  settingsBtn: document.getElementById("settings-btn") as HTMLButtonElement,
  settings: document.getElementById("settings") as HTMLDialogElement,
  settingUrl: document.getElementById("setting-url") as HTMLInputElement,
  settingKey: document.getElementById("setting-key") as HTMLInputElement,
  settingManage: document.getElementById("setting-manage") as HTMLInputElement,
};

function renderSessions(): void {
  el.sessionList.innerHTML = "";
  const sorted = [...state.sessions].sort((a, b) => b.last_activity_at - a.last_activity_at);
  for (const session of sorted.slice(0, 100)) {
    const item = document.createElement("div");
    item.className = "session-item" + (state.current?.id === session.id ? " active" : "");
    const title = session.title || session.id.slice(0, 8);
    const when = new Date(session.last_activity_at * 1000).toLocaleString();
    const main = document.createElement("div");
    main.className = "session-main";
    main.innerHTML = `<span class="title"></span><span class="when">${when}</span>`;
    main.querySelector(".title")!.textContent = title;
    item.appendChild(main);
    const actions = document.createElement("span");
    actions.className = "session-actions";
    const renameBtn = document.createElement("button");
    renameBtn.className = "icon";
    renameBtn.title = "Rename session";
    renameBtn.textContent = "✎";
    renameBtn.onclick = (event) => {
      event.stopPropagation();
      void renameSession(session);
    };
    const deleteBtn = document.createElement("button");
    deleteBtn.className = "icon danger";
    deleteBtn.title = "Delete session";
    deleteBtn.textContent = "🗑";
    deleteBtn.onclick = (event) => {
      event.stopPropagation();
      void deleteSession(session);
    };
    actions.append(renameBtn, deleteBtn);
    item.appendChild(actions);
    item.onclick = () => openSession(session);
    el.sessionList.appendChild(item);
  }
}

async function renameSession(session: SessionRow): Promise<void> {
  if (!state.client) return;
  const current = session.title || session.id.slice(0, 8);
  const next = window.prompt("Session title:", current);
  if (next === null || next.trim() === "" || next === current) return;
  try {
    await state.client.renameSession(session.id, next.trim());
    session.title = next.trim();
    if (state.current?.id === session.id) {
      state.current.title = session.title;
      el.chatTitle.textContent = session.title;
    }
    renderSessions();
  } catch (error) {
    window.alert(`Rename failed: ${error}`);
  }
}

async function deleteSession(session: SessionRow): Promise<void> {
  if (!state.client) return;
  const label = session.title || session.id.slice(0, 8);
  if (!window.confirm(`Delete session "${label}" and its transcript?`)) return;
  try {
    await state.client.deleteSession(session.id);
    state.sessions = state.sessions.filter((row) => row.id !== session.id);
    if (state.current?.id === session.id) {
      state.current = null;
      el.chatTitle.textContent = "New session";
      el.messages.innerHTML = "";
    }
    renderSessions();
  } catch (error) {
    window.alert(`Delete failed: ${error}`);
  }
}

function addMessage(role: string, content: string): HTMLElement {
  const row = document.createElement("div");
  row.className = `message ${role}`;
  const bubble = document.createElement("div");
  bubble.className = "bubble";
  bubble.textContent = content;
  row.appendChild(bubble);
  el.messages.appendChild(row);
  el.messages.scrollTop = el.messages.scrollHeight;
  return bubble;
}

async function openSession(session: SessionRow): Promise<void> {
  state.current = session;
  el.chatTitle.textContent = session.title || session.id.slice(0, 8);
  el.messages.innerHTML = "";
  renderSessions();
  try {
    const messages = await state.client!.messages(session.id);
    for (const message of messages) {
      if (message.role === "system" || !message.content) continue;
      addMessage(message.role, message.content);
    }
  } catch (error) {
    addMessage("system", `Could not load messages: ${error}`);
  }
}

async function refreshSessions(): Promise<void> {
  if (!state.client) return;
  try {
    state.sessions = await state.client.listSessions();
    renderSessions();
  } catch {
    /* gateway offline — dot already reflects it */
  }
}

async function sendTurn(): Promise<void> {
  const text = el.input.value.trim();
  if ((!text && state.pendingUploads.length === 0) || state.busy || !state.client) return;
  if (!state.current) {
    try {
      state.current = await state.client.createSession();
      state.sessions.unshift(state.current);
      el.chatTitle.textContent = state.current.title || state.current.id.slice(0, 8);
      renderSessions();
    } catch (error) {
      addMessage("system", `Could not create a session: ${error}`);
      return;
    }
  }
  // Attachments ride as hermes-style path references appended to the turn.
  const message = (text + attachmentNote()).trim();
  state.pendingUploads = [];
  renderAttachChips();
  state.busy = true;
  el.send.disabled = true;
  el.input.value = "";
  hideSlashPop();
  el.toolProgress.hidden = true;
  el.toolProgress.textContent = "";
  addMessage("user", message);
  const bubble = addMessage("assistant", "");
  bubble.classList.add("streaming");
  // Expandable tool-call cards (hermes desktop toolsets), fed by the
  // hermes.tool.started / hermes.tool.completed SSE events.
  const cards = document.createElement("div");
  cards.className = "tool-cards";
  cards.hidden = true;
  const row = bubble.parentElement!;
  el.messages.insertBefore(cards, row);
  const cardsByCallId = new Map<string, HTMLElement>();
  try {
    await state.client.chatStream(
      state.current.id,
      message,
      (chunk) => {
        bubble.textContent = (bubble.textContent || "") + chunk;
        el.messages.scrollTop = el.messages.scrollHeight;
      },
      (tool, status) => {
        el.toolProgress.textContent = `⚙ ${tool} — ${status}`;
        el.toolProgress.hidden = false;
        const running = [...cardsByCallId.values()]
          .reverse()
          .find((card) => card.classList.contains("running"));
        if (running) running.querySelector(".status")!.textContent = status;
      },
      (toolEvent) => {
        cards.hidden = false;
        if (toolEvent.kind === "started") {
          const card = document.createElement("div");
          card.className = "tool-card running";
          const head = document.createElement("div");
          head.className = "tool-card-head";
          const caret = document.createElement("span");
          caret.className = "caret";
          caret.textContent = "▶";
          const name = document.createElement("span");
          name.className = "tname";
          name.textContent = toolEvent.name || "tool";
          const status = document.createElement("span");
          status.className = "status";
          status.textContent = "running…";
          head.append(caret, name, status);
          const body = document.createElement("div");
          body.className = "tool-card-body";
          if (toolEvent.arguments) {
            const label = document.createElement("div");
            label.className = "label";
            label.textContent = "arguments";
            const pre = document.createElement("pre");
            pre.textContent = toolEvent.arguments;
            body.append(label, pre);
          }
          card.append(head, body);
          head.onclick = () => card.classList.toggle("open");
          cards.appendChild(card);
          cardsByCallId.set(toolEvent.callId, card);
        } else {
          const card = cardsByCallId.get(toolEvent.callId);
          if (card) {
            card.classList.remove("running");
            card.classList.add("done");
            card.querySelector(".status")!.textContent = "done";
            if (toolEvent.result) {
              const body = card.querySelector(".tool-card-body")!;
              const label = document.createElement("div");
              label.className = "label";
              label.textContent = "result";
              const pre = document.createElement("pre");
              pre.textContent = toolEvent.result;
              body.append(label, pre);
            }
          }
        }
        el.messages.scrollTop = el.messages.scrollHeight;
      },
    );
    bubble.classList.remove("streaming");
    await refreshSessions();
  } catch (error) {
    bubble.classList.remove("streaming");
    bubble.textContent = `error: ${error}`;
  } finally {
    state.busy = false;
    el.send.disabled = false;
    el.toolProgress.hidden = true;
    el.toolProgress.textContent = "";
    el.input.focus();
  }
}

async function pollHealth(): Promise<void> {
  if (!state.client) return;
  const ok = await state.client.health();
  el.dot.className = "dot " + (ok ? "up" : "down");
  el.dot.title = ok ? "gateway reachable" : "gateway unreachable";
  if (ok) {
    const model = await state.client.models();
    if (model) el.modelBadge.textContent = model;
  }
}

// ---------------------------------------------------------------------------
// Slash-command completion (hermes desktop-slash-commands passthrough):
// the gateway chat endpoints execute /skill + /<bundle> invocations and a
// small session command set; the popup surfaces both while typing.
// ---------------------------------------------------------------------------

const GATEWAY_SLASH_COMMANDS: [string, string][] = [
  ["/help", "gateway slash commands"],
  ["/skills", "list skills"],
  ["/tools", "list enabled tools"],
  ["/recap", "recap this session"],
  ["/title", "show or set the session title"],
  ["/usage", "this session's token usage"],
];

let slashIndex = 0;

function slashCandidates(prefix: string): { name: string; desc: string }[] {
  const builtins = GATEWAY_SLASH_COMMANDS.map(([name, desc]) => ({ name, desc }));
  const skills = state.skills.map((skill) => ({
    name: `/${skill.name}`,
    desc: skill.description || "skill",
  }));
  const lowered = prefix.toLowerCase();
  return [...builtins, ...skills].filter((item) => item.name.toLowerCase().startsWith(lowered));
}

function hideSlashPop(): void {
  el.slashPop.hidden = true;
  el.slashPop.innerHTML = "";
  slashIndex = 0;
}

function renderSlashPop(): void {
  const value = el.input.value;
  // Only while typing the leading command token (no space/newline yet).
  if (!value.startsWith("/") || /[\s]/.test(value)) {
    hideSlashPop();
    return;
  }
  const items = slashCandidates(value);
  if (items.length === 0) {
    hideSlashPop();
    return;
  }
  slashIndex = Math.min(slashIndex, items.length - 1);
  el.slashPop.innerHTML = "";
  items.forEach((item, index) => {
    const row = document.createElement("div");
    row.className = "slash-item" + (index === slashIndex ? " selected" : "");
    const name = document.createElement("span");
    name.className = "slash-name";
    name.textContent = item.name;
    const desc = document.createElement("span");
    desc.className = "slash-desc";
    desc.textContent = item.desc;
    row.append(name, desc);
    row.onmousedown = (event) => {
      event.preventDefault(); // keep composer focus
      completeSlash(item.name);
    };
    el.slashPop.appendChild(row);
  });
  el.slashPop.hidden = false;
}

function completeSlash(name: string): void {
  el.input.value = `${name} `;
  hideSlashPop();
  el.input.focus();
}

// ---------------------------------------------------------------------------
// Clipboard image paste → /api/uploads → path-reference attachments
// (hermes text-fallback media semantics: the agent inspects the cached
// file with vision_analyze/read_file).
// ---------------------------------------------------------------------------

function renderAttachChips(): void {
  el.attachChips.innerHTML = "";
  el.attachChips.hidden = state.pendingUploads.length === 0;
  state.pendingUploads.forEach((upload, index) => {
    const chip = document.createElement("span");
    chip.className = "attach-chip";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = upload.path.split("/").pop() || upload.path;
    name.title = upload.path;
    const remove = document.createElement("button");
    remove.textContent = "✕";
    remove.title = "Remove attachment";
    remove.onclick = () => {
      state.pendingUploads.splice(index, 1);
      renderAttachChips();
    };
    chip.append(name, remove);
    el.attachChips.appendChild(chip);
  });
}

async function handlePasteImages(event: ClipboardEvent): Promise<void> {
  const items = event.clipboardData?.items;
  if (!items || !state.client) return;
  for (const item of Array.from(items)) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (!file || !file.type.startsWith("image/")) continue;
    event.preventDefault();
    try {
      const upload = await state.client.uploadFile(file, `paste-${Date.now()}.png`);
      state.pendingUploads.push(upload);
      renderAttachChips();
    } catch (error) {
      addMessage("system", `Clipboard upload failed: ${error}`);
    }
  }
}

/// hermes attachment_note text-fallback format.
function attachmentNote(): string {
  if (state.pendingUploads.length === 0) return "";
  const lines = ["", "", "[Attached media]"];
  for (const upload of state.pendingUploads) {
    lines.push(`- ${upload.path} (${upload.mime}, ${upload.bytes} bytes)`);
  }
  lines.push(
    "Inspect images with vision_analyze, video with video_analyze, documents with read_file.",
  );
  return lines.join("\n");
}

async function start(): Promise<void> {
  state.bridge = await loadBridge();
  state.client = new GatewayClient(state.settings);

  // Managed gateway: start one when unreachable and management is on.
  if (state.settings.manage && state.bridge && !(await state.client.health())) {
    const binary = await state.bridge.findBinary();
    if (binary) {
      const port = await state.bridge.defaultPort();
      try {
        state.managedPid = await state.bridge.spawnGateway(binary, port);
        // Give the listener a moment to bind.
        await new Promise((resolve) => setTimeout(resolve, 1500));
      } catch (error) {
        console.warn("gateway spawn failed:", error);
      }
    }
  }

  el.newSession.onclick = async () => {
    state.current = null;
    el.chatTitle.textContent = "New session";
    el.messages.innerHTML = "";
    renderSessions();
    el.input.focus();
  };
  el.send.onclick = () => void sendTurn();
  el.input.addEventListener("keydown", (event) => {
    if (!el.slashPop.hidden) {
      const items = slashCandidates(el.input.value);
      if (event.key === "ArrowDown") {
        event.preventDefault();
        slashIndex = (slashIndex + 1) % Math.max(items.length, 1);
        renderSlashPop();
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        slashIndex = (slashIndex - 1 + items.length) % Math.max(items.length, 1);
        renderSlashPop();
        return;
      }
      if (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey)) {
        event.preventDefault();
        if (items[slashIndex]) completeSlash(items[slashIndex].name);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        hideSlashPop();
        return;
      }
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void sendTurn();
    }
  });
  el.input.addEventListener("input", () => renderSlashPop());
  el.input.addEventListener("paste", (event) => void handlePasteImages(event));
  el.settingsBtn.onclick = () => {
    el.settingUrl.value = state.settings.url;
    el.settingKey.value = state.settings.key;
    el.settingManage.checked = state.settings.manage;
    el.settings.showModal();
  };
  el.settings.addEventListener("close", () => {
    if (el.settings.returnValue !== "default") return;
    const next: GatewaySettings = {
      url: el.settingUrl.value.trim() || "http://127.0.0.1:8642",
      key: el.settingKey.value.trim(),
      manage: el.settingManage.checked,
    };
    saveSettings(next);
    state.settings = next;
    state.client = new GatewayClient(next);
    void pollHealth();
    void refreshSessions();
  });

  await pollHealth();
  await refreshSessions();
  state.skills = (await state.client.listSkills()) || [];
  setInterval(() => void pollHealth(), 10000);
  setInterval(() => void refreshSessions(), 30000);
}

// Stop a managed gateway when the window closes.
window.addEventListener("beforeunload", () => {
  if (state.bridge && state.managedPid !== null) {
    void state.bridge.stopGateway(state.managedPid);
  }
});

void start();
