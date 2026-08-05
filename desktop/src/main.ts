// ulnclaw desktop UI — session browser + streaming chat against the
// ulnclaw gateway. Tauri commands manage the gateway child process;
// everything else is plain HTTP (gateway.ts).

import { GatewayClient, loadSettings, saveSettings } from "./gateway";
import type { GatewaySettings, SessionRow } from "./gateway";

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
    const item = document.createElement("button");
    item.className = "session-item" + (state.current?.id === session.id ? " active" : "");
    const title = session.title || session.id.slice(0, 8);
    const when = new Date(session.last_activity_at * 1000).toLocaleString();
    item.innerHTML = `<span class="title"></span><span class="when">${when}</span>`;
    item.querySelector(".title")!.textContent = title;
    item.onclick = () => openSession(session);
    el.sessionList.appendChild(item);
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
  if (!text || state.busy || !state.client) return;
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
  state.busy = true;
  el.send.disabled = true;
  el.input.value = "";
  addMessage("user", text);
  const bubble = addMessage("assistant", "");
  bubble.classList.add("streaming");
  try {
    await state.client.chatStream(state.current.id, text, (chunk) => {
      bubble.textContent = (bubble.textContent || "") + chunk;
      el.messages.scrollTop = el.messages.scrollHeight;
    });
    bubble.classList.remove("streaming");
    await refreshSessions();
  } catch (error) {
    bubble.classList.remove("streaming");
    bubble.textContent = `error: ${error}`;
  } finally {
    state.busy = false;
    el.send.disabled = false;
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
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void sendTurn();
    }
  });
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
