// ulnclaw desktop UI — session browser + streaming chat against the
// ulnclaw gateway. Tauri commands manage the gateway child process;
// everything else is plain HTTP (gateway.ts).

import { GatewayClient, loadSettings, saveSettings } from "./gateway";
import type { GatewaySettings, SessionRow, SkillRow, ToolCardEvent } from "./gateway";
import { KanbanWidget } from "./kanban";
import { ProjectsWidget } from "./projects";
import { JobsWidget } from "./jobs";
import { UsageWidget } from "./usage";
import { ConfigWidget } from "./config-view";
import { DoctorWidget } from "./doctor-view";
import { WebhooksWidget } from "./webhooks";
import { RunsWidget } from "./runs";
import { SkillsWidget } from "./skills-view";
import { SessionsViewWidget } from "./sessions-view";
import { ModelsViewWidget } from "./models-view";
import { HatchOverlay } from "./hatch";
import { PetOverlay } from "./pet";
import { ModelPickerOverlay } from "./model-picker";
import { FindBar } from "./find-bar";
import { CommandPalette } from "./command-palette";
import { ArtifactsOverlay } from "./artifacts";
import { LearningOverlay } from "./learning";
import { notify, notifyError, notifySuccess } from "./notifications";
import { hideConnecting, showConnecting } from "./connecting";
import { resolveBootFailure, showBootFailure } from "./boot-failure";
import { applyStatic, fmt, onLocaleChange, t } from "./i18n";
import { LanguageSwitcher } from "./language-switcher";
import { OnboardingOverlay } from "./onboarding";
import { SessionPickerDialog } from "./session-picker";
import { clearIntro, renderIntro } from "./intro";
import { ActivityTimer, formatElapsed } from "./activity-timer";

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
  kanban: null as KanbanWidget | null,
  projects: null as ProjectsWidget | null,
  jobs: null as JobsWidget | null,
  usage: null as UsageWidget | null,
  config: null as ConfigWidget | null,
  doctor: null as DoctorWidget | null,
  webhooks: null as WebhooksWidget | null,
  runs: null as RunsWidget | null,
  skillsView: null as SkillsWidget | null,
  sessionsBrowser: null as SessionsViewWidget | null,
  modelsView: null as ModelsViewWidget | null,
  pet: null as PetOverlay | null,
  hatch: null as HatchOverlay | null,
  picker: null as ModelPickerOverlay | null,
  findBar: null as FindBar | null,
  palette: null as CommandPalette | null,
  artifacts: null as ArtifactsOverlay | null,
  learning: null as LearningOverlay | null,
  onboarding: null as OnboardingOverlay | null,
  sessionPicker: null as SessionPickerDialog | null,
  view: "chat" as "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models",
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
  settingsOnboarding: document.getElementById("settings-onboarding") as HTMLButtonElement,
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
    if (session.project) {
      const badge = document.createElement("span");
      badge.className = "session-project-badge";
      badge.title = fmt(t.session.projectBadge, { project: session.project });
      badge.textContent = session.project;
      item.appendChild(badge);
    }
    const actions = document.createElement("span");
    actions.className = "session-actions";
    const renameBtn = document.createElement("button");
    renameBtn.className = "icon";
    renameBtn.title = t.palette.renameSession;
    renameBtn.textContent = "✎";
    renameBtn.onclick = (event) => {
      event.stopPropagation();
      void renameSession(session);
    };
    const exportBtn = document.createElement("button");
    exportBtn.className = "icon";
    exportBtn.title = t.palette.exportMd;
    exportBtn.textContent = "⭳";
    exportBtn.onclick = (event) => {
      event.stopPropagation();
      void exportSession(session, "md");
    };
    const deleteBtn = document.createElement("button");
    deleteBtn.className = "icon danger";
    deleteBtn.title = t.palette.deleteSession;
    deleteBtn.textContent = "🗑";
    deleteBtn.onclick = (event) => {
      event.stopPropagation();
      void deleteSession(session);
    };
    actions.append(renameBtn, exportBtn, deleteBtn);
    item.appendChild(actions);
    item.onclick = () => openSession(session);
    el.sessionList.appendChild(item);
  }
}

async function renameSession(session: SessionRow): Promise<void> {
  if (!state.client) return;
  const current = session.title || session.id.slice(0, 8);
  const next = window.prompt(t.session.titlePrompt, current);
  if (next === null || next.trim() === "" || next === current) return;
  try {
    await state.client.renameSession(session.id, next.trim());
    session.title = next.trim();
    if (state.current?.id === session.id) {
      state.current.title = session.title;
      el.chatTitle.textContent = session.title;
    }
    renderSessions();
    notifySuccess(t.session.renamed);
  } catch (error) {
    notifyError(fmt(t.session.renameFailed, { error }));
  }
}

async function exportSession(session: SessionRow, format: "md" | "html"): Promise<void> {
  if (!state.client) return;
  try {
    const { blob, filename } = await state.client.exportSession(session.id, format);
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 5_000);
    notifySuccess(fmt(t.session.exported, { filename }));
  } catch (error) {
    notifyError(fmt(t.session.exportFailed, { error }));
  }
}

async function deleteSession(session: SessionRow): Promise<void> {
  if (!state.client) return;
  const label = session.title || session.id.slice(0, 8);
  if (!window.confirm(fmt(t.session.deleteConfirm, { label }))) return;
  try {
    await state.client.deleteSession(session.id);
    state.sessions = state.sessions.filter((row) => row.id !== session.id);
    if (state.current?.id === session.id) {
      state.current = null;
      el.chatTitle.textContent = t.session.newTitle;
      el.messages.innerHTML = "";
      renderIntro(el.messages, "new");
    }
    renderSessions();
  } catch (error) {
    notifyError(fmt(t.session.deleteFailed, { error }));
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
  refreshModelBadge();
  el.messages.innerHTML = "";
  renderSessions();
  try {
    const messages = await state.client!.messages(session.id);
    let rendered = 0;
    for (const message of messages) {
      if (message.role === "system" || !message.content) continue;
      addMessage(message.role, message.content);
      rendered += 1;
    }
    // Empty transcript → welcome copy (P255, hermes chat intro parity).
    if (rendered === 0) renderIntro(el.messages, session.id);
  } catch (error) {
    addMessage("system", fmt(t.session.loadFailed, { error }));
  }
  state.findBar?.refresh();
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
  // /resume + /sessions + /switch render the desktop session picker
  // (P254, hermes composer parity) instead of hitting the headless
  // gateway slash worker, which can't show the overlay.
  if (/^\/(resume|sessions|switch)\b/i.test(text)) {
    el.input.value = "";
    state.sessionPicker?.open();
    return;
  }
  if ((!text && state.pendingUploads.length === 0) || state.busy || !state.client) return;
  if (!state.current) {
    try {
      state.current = await state.client.createSession();
      state.sessions.unshift(state.current);
      el.chatTitle.textContent = state.current.title || state.current.id.slice(0, 8);
      refreshModelBadge();
      renderSessions();
    } catch (error) {
      addMessage("system", fmt(t.session.createFailed, { error }));
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
  clearIntro(el.messages);
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
  const cardStartedAt = new Map<string, number>();
  // Activity timer (P256, hermes activity-timer parity): elapsed ticks
  // in the progress strip — tool line when one is live, else thinking.
  const turnTimer = new ActivityTimer();
  let lastToolLine = "";
  turnTimer.start((secs) => {
    const base = lastToolLine || t.tools.thinking;
    el.toolProgress.textContent = `${base} · ${formatElapsed(secs)}`;
    el.toolProgress.hidden = false;
  });
  try {
    await state.client.chatStream(
      state.current.id,
      message,
      (chunk) => {
        bubble.textContent = (bubble.textContent || "") + chunk;
        el.messages.scrollTop = el.messages.scrollHeight;
      },
      (tool, status) => {
        lastToolLine = `⚙ ${tool} — ${status}`;
        el.toolProgress.textContent = `${lastToolLine} · ${formatElapsed(turnTimer.elapsedSeconds())}`;
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
          name.textContent = toolEvent.name || t.tools.fallbackName;
          const status = document.createElement("span");
          status.className = "status";
          status.textContent = t.tools.running;
          cardStartedAt.set(toolEvent.callId, Date.now());
          head.append(caret, name, status);
          const body = document.createElement("div");
          body.className = "tool-card-body";
          if (toolEvent.arguments) {
            const label = document.createElement("div");
            label.className = "label";
            label.textContent = t.tools.arguments;
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
            const started = cardStartedAt.get(toolEvent.callId);
            const secs = started === undefined
              ? 0
              : Math.max(0, Math.floor((Date.now() - started) / 1000));
            card.querySelector(".status")!.textContent = `${t.tools.done} · ${formatElapsed(secs)}`;
            if (toolEvent.result) {
              const body = card.querySelector(".tool-card-body")!;
              const label = document.createElement("div");
              label.className = "label";
              label.textContent = t.tools.result;
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
    bubble.textContent = fmt(t.session.errorPrefix, { error });
  } finally {
    turnTimer.stop();
    state.busy = false;
    el.send.disabled = false;
    el.toolProgress.hidden = true;
    el.toolProgress.textContent = "";
    state.findBar?.refresh();
    el.input.focus();
  }
}

/// Gateway default model (last seen via /v1/models) for the badge.
let gatewayModel = "";

/// Badge shows the session lock (🔒) when it differs from the gateway
/// default, else the gateway model; clicking opens the model picker.
function refreshModelBadge(): void {
  const locked =
    state.current?.model && state.current.model !== gatewayModel
      ? state.current.model
      : null;
  if (locked) {
    el.modelBadge.textContent = `\u{1F512} ${locked}`;
    el.modelBadge.title = t.session.modelLockTitle;
  } else {
    el.modelBadge.textContent = gatewayModel;
    el.modelBadge.title = t.session.gatewayModelTitle;
  }
  el.modelBadge.classList.toggle("locked", !!locked);
}

// P283: background watcher — polls `/v1/runs` and raises a sticky
// warning toast (plus a system notification when permitted) whenever a
// run starts waiting for approval, with an action that opens the Runs
// view. Notified run ids are remembered in localStorage so a restart
// does not re-alert for the same gate.
const APPROVAL_WATCH_KEY = "ulnclaw.approval.notified";
const APPROVAL_WATCH_INTERVAL_MS = 15_000;
const APPROVAL_WATCH_CAP = 200;

function rememberedApprovalRuns(): Set<string> {
  try {
    const raw = localStorage.getItem(APPROVAL_WATCH_KEY);
    if (raw) return new Set(JSON.parse(raw) as string[]);
  } catch {
    /* corrupted storage — start fresh */
  }
  return new Set();
}

function rememberApprovalRun(runId: string): void {
  const seen = rememberedApprovalRuns();
  seen.add(runId);
  const trimmed = [...seen].slice(-APPROVAL_WATCH_CAP);
  localStorage.setItem(APPROVAL_WATCH_KEY, JSON.stringify(trimmed));
}

async function watchApprovalsOnce(): Promise<void> {
  if (!state.client) return;
  try {
    const runs = await state.client.listRuns();
    const seen = rememberedApprovalRuns();
    for (const run of runs) {
      if (run.status !== "waiting_for_approval" || seen.has(run.run_id)) continue;
      rememberApprovalRun(run.run_id);
      const command = run.approval?.command || "";
      notify({
        kind: "warning",
        title: t.runs.approvalWaitingTitle,
        message: t.runs.approvalWaitingBody
          .replace("{id}", run.run_id.slice(0, 8))
          .replace("{command}", command),
        durationMs: 0,
        action: {
          label: t.runs.viewRuns,
          onClick: () => activeSwitchView?.("runs"),
        },
      });
      if ("Notification" in window && Notification.permission === "granted") {
        try {
          new Notification(t.runs.approvalWaitingTitle, { body: command });
        } catch {
          /* system notifications unavailable (e.g. insecure context) */
        }
      }
    }
  } catch {
    /* gateway offline — the health dot already reports it */
  }
}

function startApprovalWatcher(): void {
  window.setInterval(() => void watchApprovalsOnce(), APPROVAL_WATCH_INTERVAL_MS);
}

async function pollHealth(): Promise<void> {
  if (!state.client) return;
  const ok = await state.client.health();
  el.dot.className = "dot " + (ok ? "up" : "down");
  el.dot.title = ok ? t.session.reachable : t.session.unreachable;
  if (ok) {
    const model = await state.client.models();
    if (model) {
      gatewayModel = model;
      refreshModelBadge();
    }
  }
}

// ---------------------------------------------------------------------------
// Slash-command completion (hermes desktop-slash-commands passthrough):
// the gateway chat endpoints execute /skill + /<bundle> invocations and a
// small session command set; the popup surfaces both while typing.
// ---------------------------------------------------------------------------

function gatewaySlashCommands(): [string, string][] {
  return [
    ["/resume", t.slash.resume],
    ["/help", t.slash.help],
    ["/skills", t.slash.skills],
    ["/tools", t.slash.tools],
    ["/recap", t.slash.recap],
    ["/title", t.slash.title],
    ["/usage", t.slash.usage],
  ];
}

let slashIndex = 0;

function slashCandidates(prefix: string): { name: string; desc: string }[] {
  const builtins = gatewaySlashCommands().map(([name, desc]) => ({ name, desc }));
  const skills = state.skills.map((skill) => ({
    name: `/${skill.name}`,
    desc: skill.description || t.slash.skillFallback,
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
    remove.title = t.session.removeAttachment;
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
      addMessage("system", fmt(t.session.uploadFailed, { error }));
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

// ---------------------------------------------------------------------------
// Desktop UI bridge events (P231): a gateway spawned with
// ULNCLAW_DESKTOP=1 streams desktop-tool events over /api/desktop/events.
// Route preview.open / pane.reveal / terminal.close / message.reaction to
// the panes and answer terminal.read requests with the visible chat
// transcript (the webview's only terminal-like surface).
// ---------------------------------------------------------------------------

let desktopEventsController: AbortController | null = null;
let activeSwitchView: ((view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models") => void) | null = null;

interface DesktopEnvelope {
  session_id: string;
  event: string;
  payload: Record<string, unknown>;
}

let desktopNoticeEl: HTMLDivElement | null = null;
let desktopNoticeTimer: number | undefined;

function bridgeNotice(text: string): void {
  if (!desktopNoticeEl) {
    desktopNoticeEl = document.createElement("div");
    desktopNoticeEl.id = "desktop-notice";
    document.body.appendChild(desktopNoticeEl);
  }
  desktopNoticeEl.textContent = text;
  desktopNoticeEl.hidden = false;
  window.clearTimeout(desktopNoticeTimer);
  desktopNoticeTimer = window.setTimeout(() => {
    if (desktopNoticeEl) desktopNoticeEl.hidden = true;
  }, 4000);
}

function transcriptText(): string {
  return Array.from(el.messages.querySelectorAll(".message .bubble"))
    .map((node) => node.textContent ?? "")
    .join("\n");
}

function handleDesktopEvent(envelope: DesktopEnvelope): void {
  const payload = envelope.payload ?? {};
  const switchView = (view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models") =>
    activeSwitchView?.(view);
  switch (envelope.event) {
    case "preview.open": {
      const url = String(payload.url ?? "");
      if (url) window.open(url, "_blank");
      bridgeNotice(fmt(t.bridge.preview, { label: String(payload.label || url) }));
      break;
    }
    case "pane.reveal": {
      const pane = String(payload.pane ?? "chat");
      if (pane === "kanban" || pane === "projects" || pane === "jobs" || pane === "usage" || pane === "config" || pane === "doctor" || pane === "webhooks" || pane === "runs" || pane === "skills") {
        switchView(pane);
      } else {
        switchView("chat");
      }
      break;
    }
    case "terminal.close": {
      const running = payload.running ? t.bridge.stillRunning : "";
      bridgeNotice(fmt(t.bridge.terminalClosed, { id: String(payload.process_id ?? "?"), running }));
      break;
    }
    case "message.reaction": {
      if (state.current && envelope.session_id === state.current.id) {
        void openSession(state.current);
      }
      break;
    }
    case "terminal.read": {
      const id = String(payload.id ?? "");
      if (!id || !state.client) break;
      const lines = transcriptText().split("\n");
      const start = Number(payload.start_line ?? 0) || 0;
      const rawCount = payload.count;
      const count = rawCount != null ? Number(rawCount) : Number.NaN;
      const slice = Number.isFinite(count)
        ? lines.slice(start, start + count)
        : lines.slice(start);
      const text = slice.join("\n");
      void state.client.answerTerminalRead(
        id,
        text.length > 0,
        text.length > 0 ? text : t.bridge.terminalEmpty,
      );
      break;
    }
  }
}

function startDesktopEvents(): void {
  desktopEventsController?.abort();
  const controller = new AbortController();
  desktopEventsController = controller;
  const loop = async () => {
    for (;;) {
      if (controller.signal.aborted) return;
      const client = state.client;
      if (client) {
        try {
          await client.desktopEvents(
            (envelope) => handleDesktopEvent(envelope),
            controller.signal,
          );
        } catch {
          // aborted or gateway offline — fall through to the retry wait
        }
      }
      if (controller.signal.aborted) return;
      await new Promise((resolve) => setTimeout(resolve, 3000));
    }
  };
  void loop();
}

// ---------------------------------------------------------------------------
// Global hotkeys (P258) — the applicable subset of hermes' keybind table
// (lib/keybinds/actions.ts): composer type-to-focus, model picker, new
// session, session cycling, session picker, settings, sidebar toggle.
// ---------------------------------------------------------------------------

function anyDialogOpen(): boolean {
  return document.querySelector("dialog[open]") !== null;
}

function isTypingTarget(event: KeyboardEvent): boolean {
  const target = event.target as HTMLElement | null;
  if (!target) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

function cycleSession(direction: 1 | -1): void {
  const ids = [...state.sessions]
    .sort((a, b) => b.last_activity_at - a.last_activity_at)
    .slice(0, 100)
    .map((session) => session.id);
  if (!ids.length) return;
  const index = state.current ? ids.indexOf(state.current.id) : -1;
  const nextId = ids[(index + direction + ids.length) % ids.length];
  const session = state.sessions.find((row) => row.id === nextId);
  if (session) void openSession(session);
}

function installHotkeys(): void {
  window.addEventListener("keydown", (event) => {
    const mod = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    // mod+shift+m — model picker (hermes composer.modelPicker chord).
    if (mod && event.shiftKey && key === "m") {
      event.preventDefault();
      if (state.current) void state.picker?.open();
      return;
    }
    // mod+shift+f — session picker (hermes session.focusSearch chord).
    if (mod && event.shiftKey && key === "f") {
      event.preventDefault();
      state.sessionPicker?.open();
      return;
    }
    // mod+, — settings (hermes nav.settings chord).
    if (mod && !event.shiftKey && event.key === ",") {
      event.preventDefault();
      el.settingsBtn.click();
      return;
    }
    // mod+b — toggle sidebar (hermes view.toggleSidebar chord).
    if (mod && !event.shiftKey && key === "b") {
      event.preventDefault();
      document.getElementById("app")!.classList.toggle("no-sidebar");
      return;
    }
    if (anyDialogOpen() || isTypingTarget(event)) return;
    // mod+n / bare shift+N — new session (hermes session.new defaults).
    if (
      (mod && !event.shiftKey && !event.altKey && key === "n") ||
      (!mod && !event.ctrlKey && !event.altKey && event.key === "N")
    ) {
      event.preventDefault();
      el.newSession.click();
      return;
    }
    // ctrl+tab / ctrl+shift+tab — cycle sessions (hermes session.next/prev).
    if (event.ctrlKey && !event.metaKey && event.key === "Tab") {
      event.preventDefault();
      cycleSession(event.shiftKey ? -1 : 1);
      return;
    }
    // Type-to-focus: bare Enter focuses the composer; printable keys
    // focus and forward the character (hermes composer.focus parity —
    // '/' lands the slash-completion popup immediately).
    if (!mod && !event.altKey && event.key === "Enter") {
      event.preventDefault();
      el.input.focus();
      return;
    }
    if (!mod && !event.altKey && event.key.length === 1) {
      event.preventDefault();
      el.input.focus();
      el.input.value += event.key;
    }
  });
}

async function start(): Promise<void> {
  // Translate the static chrome for any persisted non-en locale (P251).
  applyStatic();
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
        notifyError(fmt(t.boot.spawnFailed, { error }));
      }
    }
  }

  // Cold-boot connecting overlay (hermes gateway-connecting-overlay
  // parity): show while the gateway is unreachable, poll until healthy,
  // then run the exit choreography. Never resurrects after first success.
  state.onboarding = new OnboardingOverlay(() => state.client);
  // Cold-boot health cycle: poll up to 20 s, then land on the boot-
  // failure recovery card (P253, hermes boot-failure-overlay parity).
  const bootPoll = async (): Promise<void> => {
    if (await state.client!.health()) {
      hideConnecting();
      resolveBootFailure();
      void state.onboarding!.maybeOpen();
      return;
    }
    showConnecting();
    for (let attempt = 0; attempt < 40; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      if (await state.client!.health()) {
        hideConnecting();
        resolveBootFailure();
        void state.onboarding!.maybeOpen();
        return;
      }
    }
    hideConnecting();
    showBootFailure(() => void bootPoll(), () => el.settingsBtn.click());
  };
  void bootPoll();

  el.settingsOnboarding.onclick = () => {
    el.settings.close();
    void state.onboarding!.maybeOpen(true);
  };

  el.newSession.onclick = async () => {
    state.current = null;
    el.chatTitle.textContent = t.session.newTitle;
    el.messages.innerHTML = "";
    renderIntro(el.messages, "new");
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
    startDesktopEvents();
    void pollHealth();
    void refreshSessions();
  });

  // View tabs: chat / kanban / projects / jobs.
  const chatMain = document.getElementById("chat")!;
  const kanbanMain = document.getElementById("kanban")!;
  const projectsMain = document.getElementById("projects")!;
  const jobsMain = document.getElementById("jobs")!;
  const usageMain = document.getElementById("usage")!;
  const configMain = document.getElementById("config")!;
  const doctorMain = document.getElementById("doctor")!;
  const webhooksMain = document.getElementById("webhooks")!;
  const runsMain = document.getElementById("runs")!;
  const skillsMain = document.getElementById("skills")!;
  const sessionsViewMain = document.getElementById("sessions-view")!;
  const modelsMain = document.getElementById("models")!;
  const tabChat = document.getElementById("tab-chat") as HTMLButtonElement;
  const tabKanban = document.getElementById("tab-kanban") as HTMLButtonElement;
  const tabProjects = document.getElementById("tab-projects") as HTMLButtonElement;
  const tabJobs = document.getElementById("tab-jobs") as HTMLButtonElement;
  const tabUsage = document.getElementById("tab-usage") as HTMLButtonElement;
  const tabConfig = document.getElementById("tab-config") as HTMLButtonElement;
  const tabDoctor = document.getElementById("tab-doctor") as HTMLButtonElement;
  const tabWebhooks = document.getElementById("tab-webhooks") as HTMLButtonElement;
  const tabRuns = document.getElementById("tab-runs") as HTMLButtonElement;
  const tabSkills = document.getElementById("tab-skills") as HTMLButtonElement;
  const tabSessionsView = document.getElementById("tab-sessions-view") as HTMLButtonElement;
  const tabModels = document.getElementById("tab-models") as HTMLButtonElement;
  state.kanban = new KanbanWidget(kanbanMain, () => state.client);
  state.kanban.mount();
  state.projects = new ProjectsWidget(projectsMain, () => state.client);
  state.projects.mount();
  state.jobs = new JobsWidget(jobsMain, () => state.client);
  state.jobs.mount();
  state.usage = new UsageWidget(usageMain, () => state.client);
  state.usage.mount();
  state.config = new ConfigWidget(configMain, () => state.client);
  state.config.mount();
  state.doctor = new DoctorWidget(doctorMain, () => state.client);
  state.doctor.mount();
  state.webhooks = new WebhooksWidget(webhooksMain, () => state.client);
  state.webhooks.mount();
  state.runs = new RunsWidget(runsMain, () => state.client);
  state.runs.mount();
  state.skillsView = new SkillsWidget(skillsMain, () => state.client);
  state.skillsView.mount();
  state.sessionsBrowser = new SessionsViewWidget(sessionsViewMain, () => state.client);
  state.sessionsBrowser.mount();
  state.modelsView = new ModelsViewWidget(modelsMain, () => state.client);
  state.modelsView.mount();
  const switchView = (view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models") => {
    if (view !== "chat") state.findBar?.close();
    state.view = view;
    chatMain.hidden = view !== "chat";
    kanbanMain.hidden = view !== "kanban";
    projectsMain.hidden = view !== "projects";
    jobsMain.hidden = view !== "jobs";
    usageMain.hidden = view !== "usage";
    configMain.hidden = view !== "config";
    doctorMain.hidden = view !== "doctor";
    webhooksMain.hidden = view !== "webhooks";
    runsMain.hidden = view !== "runs";
    skillsMain.hidden = view !== "skills";
    sessionsViewMain.hidden = view !== "sessions";
    modelsMain.hidden = view !== "models";
    tabChat.classList.toggle("active", view === "chat");
    tabKanban.classList.toggle("active", view === "kanban");
    tabProjects.classList.toggle("active", view === "projects");
    tabJobs.classList.toggle("active", view === "jobs");
    tabUsage.classList.toggle("active", view === "usage");
    tabConfig.classList.toggle("active", view === "config");
    tabDoctor.classList.toggle("active", view === "doctor");
    tabWebhooks.classList.toggle("active", view === "webhooks");
    tabRuns.classList.toggle("active", view === "runs");
    tabSkills.classList.toggle("active", view === "skills");
    tabSessionsView.classList.toggle("active", view === "sessions");
    tabModels.classList.toggle("active", view === "models");
    if (view === "kanban") {
      state.kanban!.start();
    } else {
      state.kanban!.stop();
    }
    if (view === "projects") {
      state.projects!.start();
    } else {
      state.projects!.stop();
    }
    if (view === "jobs") {
      state.jobs!.start();
    } else {
      state.jobs!.stop();
    }
    if (view === "usage") {
      state.usage!.start();
    } else {
      state.usage!.stop();
    }
    if (view === "config") {
      state.config!.start();
    } else {
      state.config!.stop();
    }
    if (view === "doctor") {
      state.doctor!.start();
    } else {
      state.doctor!.stop();
    }
    if (view === "webhooks") {
      state.webhooks!.start();
    } else {
      state.webhooks!.stop();
    }
    if (view === "runs") {
      state.runs!.start();
    } else {
      state.runs!.stop();
    }
    if (view === "skills") {
      state.skillsView!.start();
    } else {
      state.skillsView!.stop();
    }
    if (view === "sessions") {
      state.sessionsBrowser!.start();
    } else {
      state.sessionsBrowser!.stop();
    }
    if (view === "models") {
      state.modelsView!.start();
    } else {
      state.modelsView!.stop();
    }
  };
  tabChat.onclick = () => switchView("chat");
  tabKanban.onclick = () => switchView("kanban");
  tabProjects.onclick = () => switchView("projects");
  tabJobs.onclick = () => switchView("jobs");
  tabUsage.onclick = () => switchView("usage");
  tabConfig.onclick = () => switchView("config");
  tabDoctor.onclick = () => switchView("doctor");
  tabWebhooks.onclick = () => switchView("webhooks");
  tabRuns.onclick = () => switchView("runs");
  tabSkills.onclick = () => switchView("skills");
  tabSessionsView.onclick = () => switchView("sessions");
  tabModels.onclick = () => switchView("models");

  // Desktop bridge events (P231) — see startDesktopEvents.
  activeSwitchView = switchView;
  startDesktopEvents();

  // Petdex mascot overlay (display.pet.* driven, polls the gateway).
  state.pet = new PetOverlay(() => state.client);
  state.pet.start();

  // Desktop hatch overlay (gateway hatch jobs; hermes pet-generate parity).
  const hatchBtn = document.createElement("button");
  hatchBtn.id = "hatch-btn";
  hatchBtn.className = "ghost";
  hatchBtn.textContent = t.chrome.hatchPet;
  const settingsBtn = document.getElementById("settings-btn")!;
  settingsBtn.parentElement!.insertBefore(hatchBtn, settingsBtn);

  // Language switcher (P251, hermes language-switcher parity): globe
  // trigger in the sidebar footer with a searchable locale popover.
  new LanguageSwitcher(settingsBtn.parentElement!);
  state.hatch = new HatchOverlay(() => state.client, () => state.pet?.refresh());
  hatchBtn.onclick = () => state.hatch!.open();

  // Per-session model picker (hermes model-picker parity): the badge
  // opens the overlay; locks ride POST /api/sessions/:id/model.
  state.picker = new ModelPickerOverlay(
    () => state.client,
    () => state.current?.id ?? null,
    () => {
      const m = state.current?.model;
      return m && m !== gatewayModel ? m : null;
    },
    (selection) => {
      if (state.current) {
        state.current.model =
          selection.model === gatewayModel ? null : selection.model;
        refreshModelBadge();
      }
    },
    () => void state.onboarding?.maybeOpen(true),
  );
  el.modelBadge.addEventListener("click", () => {
    if (!state.current || !state.picker) return;
    void state.picker.open();
  });

  // Find-in-chat bar (hermes find-bar parity, DOM-based for the Tauri
  // webview): Ctrl/Cmd+F opens over the chat view, Enter steps.
  state.findBar = new FindBar(chatMain, el.messages, () => state.view === "chat");

  // Learning view (hermes star-map parity): learned skills + memory
  // graph over /api/learning/*.
  state.learning = new LearningOverlay(() => state.client);

  // Artifacts browser (hermes artifacts-view parity): scans recent
  // transcripts for links/files/images.
  state.artifacts = new ArtifactsOverlay(
    () => state.client,
    () => state.sessions,
    (id) => {
      const session = state.sessions.find((s) => s.id === id);
      if (session) void openSession(session);
    },
  );

  // Command palette (hermes command-palette parity): Ctrl/Cmd+K fuzzy
  // launcher for views, session switching, and session actions.
  // Session picker (P254): opened by /resume|/sessions|/switch and the
  // command palette; resumes the picked session.
  state.sessionPicker = new SessionPickerDialog({
    sessions: () => state.sessions,
    currentSessionId: () => state.current?.id ?? null,
    openSession: async (id) => {
      const session = state.sessions.find((row) => row.id === id);
      if (session) await openSession(session);
    },
  });

  state.palette = new CommandPalette({
    sessions: () => state.sessions,
    currentSessionId: () => state.current?.id ?? null,
    newSession: () => el.newSession.click(),
    openSession: async (id) => {
      const session = state.sessions.find((s) => s.id === id);
      if (session) await openSession(session);
    },
    renameSession: async () => {
      if (state.current) await renameSession(state.current);
    },
    deleteSession: async () => {
      if (state.current) await deleteSession(state.current);
    },
    exportSession: async (format) => {
      if (state.current) await exportSession(state.current, format);
    },
    modelPicker: () => state.picker?.open() ?? Promise.resolve(),
    resumeSession: () => state.sessionPicker?.open() ?? Promise.resolve(),
    artifacts: () => state.artifacts?.open() ?? Promise.resolve(),
    learning: () => state.learning?.open() ?? Promise.resolve(),
    findInChat: () => {
      switchView("chat");
      state.findBar?.open();
    },
    switchView,
    openSettings: () => el.settingsBtn.click(),
    refreshSessions: () => refreshSessions(),
  });

  // Translate widget skeletons mounted after the initial applyStatic
  // (idempotent — covers non-en persisted locales on cold boot).
  applyStatic();

  // Locale switch re-render (P251): static attributes are handled by
  // setLocale -> applyStatic; refresh the dynamic chrome and views.
  onLocaleChange(() => {
    renderSessions();
    refreshModelBadge();
    hatchBtn.textContent = t.chrome.hatchPet;
    state.kanban?.rerender();
    state.projects?.rerender();
    state.jobs?.rerender();
    state.findBar?.rerender();
  });

  installHotkeys();

  await pollHealth();
  await refreshSessions();
  state.skills = (await state.client.listSkills()) || [];
  setInterval(() => void pollHealth(), 10000);
  setInterval(() => void refreshSessions(), 30000);
  startApprovalWatcher();
}

// Stop a managed gateway when the window closes.
window.addEventListener("beforeunload", () => {
  if (state.bridge && state.managedPid !== null) {
    void state.bridge.stopGateway(state.managedPid);
  }
});

void start();
