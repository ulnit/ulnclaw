// ulnclaw desktop UI — session browser + streaming chat against the
// ulnclaw gateway. Tauri commands manage the gateway child process;
// everything else is plain HTTP (gateway.ts).

import { GatewayClient, loadSettings, saveSettings } from "./gateway";
import type { FsEntry, GatewaySettings, SessionRow, SkillRow, ToolCardEvent } from "./gateway";
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
import { PluginsViewWidget } from "./plugins-view";
import { PairingViewWidget } from "./pairing-view";
import { HatchOverlay } from "./hatch";
import { PetOverlay } from "./pet";
import { ModelPickerOverlay } from "./model-picker";
import { FindBar } from "./find-bar";
import { CommandPalette } from "./command-palette";
import { ArtifactsOverlay } from "./artifacts";
import { LearningOverlay } from "./learning";
import { notify, notifyError, notifySuccess, notificationHistory, notificationUnread, markNotificationsRead, clearNotificationHistory, onNotificationHistoryChange } from "./notifications";
import { hideConnecting, showConnecting } from "./connecting";
import { resolveBootFailure, showBootFailure } from "./boot-failure";
import { applyStatic, currentLocale, fmt, onLocaleChange, t } from "./i18n";
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
  /** P368: session id -> total tokens from /api/usage (sidebar badges). */
  sessionTokens: new Map<string, number>(),
  /** P372: sidebar session-filter text (title or id substring). */
  sessionFilterText: "",
  /** P378: sent-prompt history for terminal-style ↑/↓ recall. */
  composerHistory: [] as string[],
  composerHistoryIndex: null as number | null,
  composerDraft: "",
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
  pluginsView: null as PluginsViewWidget | null,
  pairingView: null as PairingViewWidget | null,
  pet: null as PetOverlay | null,
  hatch: null as HatchOverlay | null,
  picker: null as ModelPickerOverlay | null,
  findBar: null as FindBar | null,
  palette: null as CommandPalette | null,
  artifacts: null as ArtifactsOverlay | null,
  learning: null as LearningOverlay | null,
  onboarding: null as OnboardingOverlay | null,
  sessionPicker: null as SessionPickerDialog | null,
  view: "chat" as "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing",
};

const el = {
  dot: document.getElementById("gateway-dot")!,
  statusbar: document.getElementById("statusbar")!,
  sessionList: document.getElementById("session-list")!,
  sessionFilter: document.getElementById("session-filter") as HTMLInputElement,
  newSession: document.getElementById("new-session") as HTMLButtonElement,
  messages: document.getElementById("messages")!,
  scrollBottom: document.getElementById("scroll-bottom") as HTMLButtonElement,
  chatTitle: document.getElementById("chat-title")!,
  sessionInfoDialog: document.getElementById("session-info-dialog") as HTMLDialogElement,
  sessionInfoRows: document.getElementById("session-info-rows")!,
  sessionInfoCopy: document.getElementById("session-info-copy") as HTMLButtonElement,
  modelBadge: document.getElementById("model-badge")!,
  contextMeter: document.getElementById("context-meter")!,
  contextMeterFill: document.getElementById("context-meter-fill")!,
  contextMeterText: document.getElementById("context-meter-text")!,
  dayJump: document.getElementById("day-jump") as HTMLSelectElement,
  chatActions: document.getElementById("chat-header-actions")!,
  chatRename: document.getElementById("chat-rename") as HTMLButtonElement,
  chatExport: document.getElementById("chat-export") as HTMLButtonElement,
  chatDelete: document.getElementById("chat-delete") as HTMLButtonElement,
  input: document.getElementById("input") as HTMLTextAreaElement,
  send: document.getElementById("send") as HTMLButtonElement,
  mic: document.getElementById("mic") as HTMLButtonElement,
  toolProgress: document.getElementById("tool-progress")!,
  slashPop: document.getElementById("slash-pop")!,
  attachChips: document.getElementById("attach-chips")!,
  settingsBtn: document.getElementById("settings-btn") as HTMLButtonElement,
  notifyBell: document.getElementById("notify-bell") as HTMLButtonElement,
  notifyBadge: document.getElementById("notify-badge")!,
  notifyHistoryDialog: document.getElementById("notify-history-dialog") as HTMLDialogElement,
  notifyHistoryList: document.getElementById("notify-history-list")!,
  notifyHistoryClear: document.getElementById("notify-history-clear") as HTMLButtonElement,
  settings: document.getElementById("settings") as HTMLDialogElement,
  settingUrl: document.getElementById("setting-url") as HTMLInputElement,
  settingKey: document.getElementById("setting-key") as HTMLInputElement,
  settingManage: document.getElementById("setting-manage") as HTMLInputElement,
  settingTheme: document.getElementById("setting-theme") as HTMLSelectElement,
  settingFont: document.getElementById("setting-font") as HTMLSelectElement,
  settingsOnboarding: document.getElementById("settings-onboarding") as HTMLButtonElement,
  settingsShortcuts: document.getElementById("settings-shortcuts") as HTMLButtonElement,
  shortcutsDialog: document.getElementById("shortcuts-dialog") as HTMLDialogElement,
  shortcutsTable: document.getElementById("shortcuts-table")!,
  settingsRestart: document.getElementById("settings-restart") as HTMLButtonElement,
  attachFile: document.getElementById("attach-file") as HTMLButtonElement,
  fsDialog: document.getElementById("fs-dialog") as HTMLDialogElement,
  fsUp: document.getElementById("fs-up") as HTMLButtonElement,
  fsPath: document.getElementById("fs-path") as HTMLInputElement,
  fsGo: document.getElementById("fs-go") as HTMLButtonElement,
  fsEntries: document.getElementById("fs-entries")!,
  fsStatus: document.getElementById("fs-status")!,
  fsClose: document.getElementById("fs-close") as HTMLButtonElement,
  fsMkdir: document.getElementById("fs-mkdir") as HTMLButtonElement,
  fsGitRoot: document.getElementById("fs-git-root") as HTMLButtonElement,
  fsPreviewDialog: document.getElementById("fs-preview-dialog") as HTMLDialogElement,
  fsPreviewTitle: document.getElementById("fs-preview-title")!,
  fsPreviewText: document.getElementById("fs-preview-text") as HTMLTextAreaElement,
  fsPreviewStatus: document.getElementById("fs-preview-status")!,
  fsPreviewSave: document.getElementById("fs-preview-save") as HTMLButtonElement,
  fsPreviewClose: document.getElementById("fs-preview-close") as HTMLButtonElement,
};

/** P364: compact relative time for sidebar session rows (full
 * timestamp stays in the tooltip). */
function sessionWhen(epochSeconds: number): string {
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - epochSeconds));
  if (secs < 60) return t.session.whenNow;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d`;
  return new Date(epochSeconds * 1000).toLocaleDateString();
}

function renderSessions(): void {
  el.sessionList.innerHTML = "";
  // P372: optional sidebar filter (title or id substring, case-insensitive).
  const filter = state.sessionFilterText.trim().toLowerCase();
  const sorted = [...state.sessions]
    .sort((a, b) => b.last_activity_at - a.last_activity_at)
    .filter((session) => {
      if (!filter) return true;
      const title = session.title || session.id.slice(0, 8);
      return (
        title.toLowerCase().includes(filter) ||
        session.id.toLowerCase().includes(filter)
      );
    });
  for (const session of sorted.slice(0, 100)) {
    const item = document.createElement("div");
    item.className = "session-item" + (state.current?.id === session.id ? " active" : "");
    const title = session.title || session.id.slice(0, 8);
    const whenFull = new Date(session.last_activity_at * 1000).toLocaleString();
    const main = document.createElement("div");
    main.className = "session-main";
    main.innerHTML = `<span class="title"></span><span class="when"></span>`;
    main.querySelector(".title")!.textContent = title;
    const whenEl = main.querySelector<HTMLSpanElement>(".when")!;
    whenEl.textContent = session.message_count
      ? `${sessionWhen(session.last_activity_at)} · ${session.message_count}`
      : sessionWhen(session.last_activity_at);
    whenEl.title = whenFull;
    const tokens = state.sessionTokens.get(session.id) ?? 0;
    if (tokens > 0) {
      whenEl.textContent += ` · ${formatTokens(tokens)} tok`;
      whenEl.title = `${whenFull} · ${fmt(t.session.tokensTitle, { tokens: tokens.toLocaleString() })}`;
    }
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
  // P373: empty-state hint when the filter matches nothing.
  if (sorted.length === 0 && filter) {
    const empty = document.createElement("div");
    empty.className = "session-filter-empty";
    empty.textContent = t.session.filterNoMatch;
    el.sessionList.appendChild(empty);
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

/** P363: smart auto-scroll — stick to the bottom only while the user
 * is near it; otherwise a floating ⭳ button jumps back down. */
const SCROLL_STICK_PX = 120;

function chatNearBottom(): boolean {
  const box = el.messages;
  return box.scrollHeight - box.scrollTop - box.clientHeight < SCROLL_STICK_PX;
}

function maybeScrollToBottom(): void {
  if (chatNearBottom()) {
    el.messages.scrollTop = el.messages.scrollHeight;
    el.scrollBottom.hidden = true;
  } else {
    el.scrollBottom.hidden = false;
  }
}

function addMessage(role: string, content: string): HTMLElement {
  const row = document.createElement("div");
  row.className = `message ${role}`;
  const bubble = document.createElement("div");
  bubble.className = "bubble";
  bubble.textContent = content;
  row.appendChild(bubble);
  // Message actions (P344 read-aloud on assistant replies; P361 copy
  // on user + assistant bubbles).
  if (content.trim() && (role === "assistant" || role === "user")) {
    const actions = document.createElement("div");
    actions.className = "msg-actions";
    const copy = document.createElement("button");
    copy.className = "ghost msg-copy";
    copy.textContent = "\u29C9";
    copy.title = t.session.copyTitle;
    copy.onclick = () => {
      void navigator.clipboard.writeText(content).then(
        () => {
          copy.textContent = "\u2713";
          window.setTimeout(() => {
            copy.textContent = "\u29C9";
          }, 1200);
        },
        () => notifyError(t.session.copyFailed),
      );
    };
    actions.appendChild(copy);
    if (role === "assistant") {
      const speak = document.createElement("button");
      speak.className = "ghost msg-speak";
      speak.textContent = "\u{1F50A}";
      speak.title = t.session.speakTitle;
      speak.onclick = () => void speakMessage(content, speak);
      actions.appendChild(speak);
    }
    row.appendChild(actions);
  }
  el.messages.appendChild(row);
  maybeScrollToBottom();
  return bubble;
}

/** Currently playing TTS audio (one at a time; P344). */
let currentSpeech: { audio: HTMLAudioElement; button: HTMLButtonElement } | null = null;

async function speakMessage(text: string, button: HTMLButtonElement): Promise<void> {
  if (!state.client) return;
  if (currentSpeech) {
    currentSpeech.audio.pause();
    currentSpeech.button.textContent = "\u{1F50A}";
    const same = currentSpeech.button === button;
    currentSpeech = null;
    if (same) return;
  }
  button.disabled = true;
  try {
    const dataUrl = await state.client.audioSpeak(text);
    const audio = new Audio(dataUrl);
    currentSpeech = { audio, button };
    audio.onended = () => {
      button.textContent = "\u{1F50A}";
      if (currentSpeech?.audio === audio) currentSpeech = null;
    };
    button.textContent = "\u23F9";
    await audio.play();
  } catch (error) {
    notifyError(fmt(t.session.speakFailed, { error }));
  } finally {
    button.disabled = false;
  }
}

// Chat day dividers (P367, hermes transcript parity): calendar-day
// boundary markers rendered from the per-message stored timestamps.
const LOCALE_DATE_TAGS: Record<string, string> = {
  en: "en-US", zh: "zh-CN", "zh-hant": "zh-TW", ja: "ja-JP", ar: "ar",
};

function dayKey(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function sameCalendarDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function dayLabel(timestamp: number, options: Intl.DateTimeFormatOptions): string {
  const date = new Date(timestamp * 1000);
  if (sameCalendarDay(date, new Date())) return t.session.dayToday;
  return date.toLocaleDateString(LOCALE_DATE_TAGS[currentLocale()] ?? "en-US", options);
}

function addDayDivider(timestamp: number): void {
  const label = dayLabel(timestamp, { year: "numeric", month: "long", day: "numeric" });
  const divider = document.createElement("div");
  divider.className = "day-divider";
  divider.dataset.day = dayKey(timestamp);
  const text = document.createElement("span");
  text.className = "day-divider-label";
  text.textContent = label;
  divider.appendChild(text);
  el.messages.appendChild(divider);
}

async function openSession(session: SessionRow): Promise<void> {
  state.current = session;
  el.chatTitle.textContent = session.title || session.id.slice(0, 8);
  refreshModelBadge();
  el.messages.innerHTML = "";
  el.dayJump.hidden = true;
  el.chatActions.hidden = false;
  renderSessions();
  try {
    const messages = await state.client!.messages(session.id, { timestamps: true });
    let rendered = 0;
    let lastDay = "";
    const days: { key: string; timestamp: number }[] = [];
    for (const message of messages) {
      if (message.role === "system" || !message.content) continue;
      if (typeof message.timestamp === "number" && message.timestamp > 0) {
        const day = dayKey(message.timestamp);
        if (day !== lastDay) {
          addDayDivider(message.timestamp);
          days.push({ key: day, timestamp: message.timestamp });
          lastDay = day;
        }
      }
      addMessage(message.role, message.content);
      rendered += 1;
    }
    // Empty transcript → welcome copy (P255, hermes chat intro parity).
    if (rendered === 0) renderIntro(el.messages, session.id);
    renderDayJump(days);
  } catch (error) {
    addMessage("system", fmt(t.session.loadFailed, { error }));
  }
  state.findBar?.refresh();
  void updateContextMeter();
}

/** P375: chat-header day jump — transcripts spanning two or more days
 * list their days in a dropdown that scrolls to the picked divider. */
function renderDayJump(days: { key: string; timestamp: number }[]): void {
  el.dayJump.innerHTML = "";
  if (days.length < 2) {
    el.dayJump.hidden = true;
    return;
  }
  for (const day of days) {
    const option = document.createElement("option");
    option.value = day.key;
    option.textContent = dayLabel(day.timestamp, { month: "short", day: "numeric" });
    el.dayJump.appendChild(option);
  }
  el.dayJump.hidden = false;
}

async function refreshSessions(): Promise<void> {
  if (!state.client) return;
  try {
    state.sessions = await state.client.listSessions();
    await refreshSessionTokens();
    renderSessions();
  } catch {
    /* gateway offline — dot already reflects it */
  }
}

/** P368: per-session token badges — map session id -> total tokens
 * from /api/usage so renderSessions can annotate sidebar rows. */
async function refreshSessionTokens(): Promise<void> {
  if (!state.client) return;
  try {
    const usage = await state.client.usage(500);
    const next = new Map<string, number>();
    for (const row of usage.sessions) next.set(row.id, row.total_tokens);
    state.sessionTokens = next;
  } catch {
    /* gateway offline — keep the last badges */
  }
}

// Voice input (P327, hermes desktop voice parity): MediaRecorder capture
// -> POST /api/audio/transcribe -> transcript appended to the composer.
let mediaRecorder: MediaRecorder | null = null;
let mediaChunks: Blob[] = [];

function setMicState(recording: boolean): void {
  el.mic.classList.toggle("recording", recording);
  el.mic.title = recording ? t.chrome.micRecording : t.chrome.micTitle;
}

async function toggleMic(): Promise<void> {
  if (mediaRecorder && mediaRecorder.state !== "inactive") {
    mediaRecorder.stop();
    return;
  }
  if (!state.client) return;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    mediaChunks = [];
    const recorder = new MediaRecorder(stream);
    recorder.addEventListener("dataavailable", (event) => {
      if (event.data.size > 0) mediaChunks.push(event.data);
    });
    recorder.addEventListener("stop", () => {
      stream.getTracks().forEach((track) => track.stop());
      setMicState(false);
      void transcribeRecording(recorder.mimeType);
    });
    recorder.start();
    mediaRecorder = recorder;
    setMicState(true);
  } catch (error) {
    setMicState(false);
    notifyError(fmt(t.chrome.micFailed, { error: String(error) }));
  }
}

async function transcribeRecording(mimeType: string): Promise<void> {
  const client = state.client;
  if (!client || mediaChunks.length === 0) return;
  const type = mimeType || "audio/webm";
  const blob = new Blob(mediaChunks, { type });
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  const chunkSize = 32_768;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  const dataUrl = `data:${type};base64,${btoa(binary)}`;
  try {
    const result = await client.audioTranscribe(dataUrl, type);
    const transcript = result.transcript.trim();
    if (transcript) {
      el.input.value = el.input.value ? `${el.input.value} ${transcript}` : transcript;
      el.input.focus();
    }
  } catch (error) {
    notifyError(fmt(t.chrome.micFailed, { error: String(error) }));
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
  // /new starts a fresh session straight from the composer (P374,
  // hermes CLI /new parity).
  if (/^\/new\b/i.test(text)) {
    el.input.value = "";
    el.newSession.click();
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
  // P378: record the prompt for ↑/↓ recall.
  if (text) {
    state.composerHistory.push(text);
    if (state.composerHistory.length > 50) state.composerHistory.shift();
  }
  state.composerHistoryIndex = null;
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
        maybeScrollToBottom();
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
        maybeScrollToBottom();
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
    void updateContextMeter();
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
    // P365: enrich the dot tooltip with the detailed open probe.
    const detailed = await state.client.healthDetailed();
    if (detailed) {
      el.dot.title = [
        `${detailed.service} v${detailed.version}`,
        `${detailed.provider}/${detailed.model}`,
        detailed.auth_required ? t.chrome.dotAuthOn : t.chrome.dotAuthOff,
        t.chrome.dotRuns.replace("{count}", String(detailed.runs_tracked)),
      ].join(" · ");
    }
    void updateStatusBar();
  } else {
    el.statusbar.hidden = true;
  }
}

function formatUptime(secs: number): string {
  const days = Math.floor(secs / 86400);
  const hours = Math.floor((secs % 86400) / 3600);
  const mins = Math.floor((secs % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${mins}m`;
  if (mins > 0) return `${mins}m`;
  return `${secs}s`;
}

/** P355: shell status bar — live gateway facts along the bottom edge
 * (version/os/uptime, gateway model, sessions/runs/plugins census).
 * Clicking it opens the Doctor view. */
async function updateStatusBar(): Promise<void> {
  if (!state.client) {
    el.statusbar.hidden = true;
    return;
  }
  try {
    const [info, usage] = await Promise.all([
      state.client.systemInfo(),
      // P370: process token/tool census for the status bar; keep the
      // payload small — only the summary cards matter here.
      state.client.usage(1).catch(() => null),
    ]);
    const segs: string[] = [
      `v${info.version} · ${info.os}/${info.arch} · ${t.chrome.statusUp.replace("{duration}", formatUptime(info.uptime_secs))}`,
    ];
    if (gatewayModel) segs.push(gatewayModel);
    segs.push(t.chrome.statusSessions.replace("{count}", String(info.sessions)));
    segs.push(t.chrome.statusRuns.replace("{count}", String(info.active_runs)));
    segs.push(t.chrome.statusPlugins.replace("{count}", String(info.plugins_loaded)));
    if (usage) {
      segs.push(
        t.chrome.statusTokens
          .replace("{tokens}", formatTokens(usage.process.total_tokens))
          .replace("{calls}", String(usage.process.tool_calls)),
      );
    }
    el.statusbar.innerHTML = segs
      .map((seg) => `<span class="statusbar-seg">${seg}</span>`)
      .join("");
    el.statusbar.hidden = false;
  } catch {
    el.statusbar.hidden = true;
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
    ["/kanban", t.slash.kanban],
    ["/new", t.slash.newSession],
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

/** P358: drag & drop attach — dropping files anywhere on the chat pane
 * uploads them through the same /api/uploads path as clipboard paste
 * (any file kind; the attachment note carries path references). */
function installDragDrop(): void {
  const zone = document.getElementById("chat")!;
  let dragDepth = 0;
  const hasFiles = (event: DragEvent): boolean =>
    !!event.dataTransfer && Array.from(event.dataTransfer.types).includes("Files");
  zone.addEventListener("dragenter", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragDepth += 1;
    zone.classList.add("drag-over");
  });
  zone.addEventListener("dragover", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
  });
  zone.addEventListener("dragleave", (event) => {
    if (!hasFiles(event)) return;
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) zone.classList.remove("drag-over");
  });
  zone.addEventListener("drop", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragDepth = 0;
    zone.classList.remove("drag-over");
    const files = event.dataTransfer ? Array.from(event.dataTransfer.files) : [];
    if (files.length === 0 || !state.client) return;
    void (async () => {
      for (const file of files) {
        try {
          const upload = await state.client!.uploadFile(
            file,
            file.name || `drop-${Date.now()}`,
          );
          state.pendingUploads.push(upload);
          renderAttachChips();
        } catch (error) {
          addMessage("system", fmt(t.session.uploadFailed, { error }));
        }
      }
    })();
  });
}

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

// ---------------------------------------------------------------------------
// Dashboard appearance — theme/font persistence over /api/dashboard/*
// (P331, hermes parity). The gateway stores the selection; the shell owns
// the palettes (style.css data-theme/data-font blocks).
// ---------------------------------------------------------------------------

const FONT_IDS = [
  "theme", "system-sans", "system-serif", "system-mono", "inter",
  "ibm-plex-sans", "work-sans", "atkinson-hyperlegible", "dm-sans",
  "spectral", "fraunces", "source-serif", "jetbrains-mono",
  "ibm-plex-mono", "space-mono",
];

function applyTheme(name: string): void {
  document.documentElement.dataset.theme = name;
}

function applyFont(font: string): void {
  if (font === "theme") delete document.documentElement.dataset.font;
  else document.documentElement.dataset.font = font;
}

async function loadAppearance(): Promise<void> {
  if (!state.client) return;
  try {
    const payload = await state.client.dashboardThemes();
    el.settingTheme.innerHTML = "";
    for (const theme of payload.themes) {
      const option = document.createElement("option");
      option.value = theme.name;
      option.textContent = theme.label;
      option.title = theme.description;
      el.settingTheme.appendChild(option);
    }
    el.settingTheme.value = payload.active;
    applyTheme(payload.active);
  } catch {
    /* gateway without P331 endpoints — keep the default look */
  }
  try {
    const font = await state.client.dashboardFont();
    el.settingFont.innerHTML = "";
    for (const id of FONT_IDS) {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = id;
      el.settingFont.appendChild(option);
    }
    el.settingFont.value = FONT_IDS.includes(font) ? font : "theme";
    applyFont(font);
  } catch {
    /* see above */
  }
}

// ---------------------------------------------------------------------------
// Gateway filesystem picker -> /api/fs/* -> /api/uploads (P329, hermes parity)
// ---------------------------------------------------------------------------

async function openFsPicker(): Promise<void> {
  if (!state.client) return;
  el.fsStatus.textContent = "";
  el.fsEntries.innerHTML = "";
  el.fsDialog.showModal();
  try {
    const { cwd } = await state.client.fsDefaultCwd();
    await renderFsEntries(cwd);
  } catch (error) {
    el.fsStatus.textContent = fmt(t.chrome.fsFailed, { error });
  }
}

async function renderFsEntries(path: string): Promise<void> {
  if (!state.client) return;
  el.fsPath.value = path;
  el.fsStatus.textContent = "";
  el.fsEntries.innerHTML = "";
  let entries: FsEntry[];
  try {
    ({ entries } = await state.client.fsList(path));
  } catch (error) {
    el.fsStatus.textContent = fmt(t.chrome.fsFailed, { error });
    return;
  }
  if (entries.length === 0) {
    el.fsStatus.textContent = t.chrome.fsEmpty;
    return;
  }
  for (const entry of entries) {
    const row = document.createElement("div");
    row.className = "fs-entry-row";
    const main = document.createElement("button");
    main.className = "fs-entry" + (entry.isDirectory ? " dir" : "");
    main.textContent = (entry.isDirectory ? "\u{1F4C1} " : "\u{1F4C4} ") + entry.name;
    main.title = entry.path;
    main.onclick = () => {
      if (entry.isDirectory) void renderFsEntries(entry.path);
      else void pickFsFile(entry);
    };
    row.appendChild(main);
    if (!entry.isDirectory) {
      const download = document.createElement("button");
      download.className = "ghost fs-entry-download";
      download.textContent = "\u2B07";
      download.title = t.chrome.fsDownloadTitle;
      download.onclick = () => {
        if (state.client) window.open(state.client.fsDownloadUrl(entry.path), "_blank");
      };
      row.appendChild(download);
      const preview = document.createElement("button");
      preview.className = "ghost fs-entry-download";
      preview.textContent = "\u{1F441}";
      preview.title = t.chrome.fsPreviewOpen;
      preview.onclick = () => void openFsPreview(entry.path);
      row.appendChild(preview);
    }
    el.fsEntries.appendChild(row);
  }
}

async function pickFsFile(entry: FsEntry): Promise<void> {
  if (!state.client) return;
  try {
    const dataUrl = await state.client.fsReadDataUrl(entry.path);
    const response = await fetch(dataUrl);
    const blob = await response.blob();
    const file = new File([blob], entry.name, { type: blob.type || "application/octet-stream" });
    const upload = await state.client.uploadFile(file, entry.name);
    state.pendingUploads.push(upload);
    renderAttachChips();
    el.fsDialog.close();
  } catch (error) {
    el.fsStatus.textContent = fmt(t.chrome.fsFailed, { error });
    addMessage("system", fmt(t.session.uploadFailed, { error }));
  }
}

// Text-file preview/edit over /api/fs/read-text + write-text (P347).
let fsPreviewPath = "";

async function openFsPreview(path: string): Promise<void> {
  if (!state.client) return;
  fsPreviewPath = path;
  el.fsPreviewTitle.textContent = path.split("/").pop() || path;
  el.fsPreviewTitle.title = path;
  el.fsPreviewText.value = "";
  el.fsPreviewText.readOnly = true;
  el.fsPreviewSave.disabled = true;
  el.fsPreviewStatus.textContent = t.chrome.fsPreviewLoading;
  el.fsPreviewDialog.showModal();
  try {
    const preview = await state.client.fsReadText(path);
    if (preview.binary) {
      el.fsPreviewStatus.textContent = t.chrome.fsPreviewBinary;
      return;
    }
    el.fsPreviewText.value = preview.text;
    if (preview.truncated) {
      el.fsPreviewStatus.textContent = t.chrome.fsPreviewTruncated;
    } else {
      el.fsPreviewStatus.textContent = "";
      el.fsPreviewText.readOnly = false;
      el.fsPreviewSave.disabled = false;
    }
  } catch (error) {
    el.fsPreviewStatus.textContent = fmt(t.chrome.fsPreviewFailed, { error });
  }
}

async function saveFsPreview(): Promise<void> {
  if (!state.client || !fsPreviewPath || el.fsPreviewSave.disabled) return;
  try {
    await state.client.fsWriteText(fsPreviewPath, el.fsPreviewText.value);
    el.fsPreviewStatus.textContent = t.chrome.fsPreviewSaved;
    setTimeout(() => el.fsPreviewDialog.close(), 500);
  } catch (error) {
    el.fsPreviewStatus.textContent = fmt(t.chrome.fsPreviewSaveFailed, { error });
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
let activeSwitchView: ((view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing") => void) | null = null;

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
  const switchView = (view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing") =>
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
    // F1 — keyboard shortcuts cheatsheet (works over dialogs/inputs).
    if (event.key === "F1") {
      event.preventDefault();
      if (el.shortcutsDialog.open) el.shortcutsDialog.close();
      else openShortcuts();
      return;
    }
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

/** P343: restart the managed gateway child — stop, respawn, wait for
 * health, then refresh the session list. Only available when the shell
 * actually manages the gateway (Tauri + manage toggle). */
async function restartGateway(): Promise<boolean> {
  if (!state.bridge || state.managedPid === null) {
    notifyError(t.chrome.restartUnavailable);
    return false;
  }
  const bridge = state.bridge;
  try {
    await bridge.stopGateway(state.managedPid);
  } catch {
    // Best effort: a stale pid still clears below.
  }
  state.managedPid = null;
  await new Promise((resolve) => setTimeout(resolve, 800));
  const binary = await bridge.findBinary();
  if (!binary) {
    notifyError(t.chrome.restartUnavailable);
    return false;
  }
  const port = await bridge.defaultPort();
  try {
    state.managedPid = await bridge.spawnGateway(binary, port);
  } catch (error) {
    notifyError(fmt(t.boot.spawnFailed, { error }));
    return false;
  }
  for (let attempt = 0; attempt < 20; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 500));
    if (state.client && (await state.client.health())) {
      notifySuccess(t.chrome.restartDone);
      await refreshSessions();
      return true;
    }
  }
  notifyError(t.chrome.restartFailed);
  return false;
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
  void loadAppearance();

  // P343: gateway restart affordance — only when the shell manages the
  // gateway child (browser-tab mode has nothing to restart).
  el.settingsRestart.hidden = !(state.bridge && state.settings.manage);
  el.settingsRestart.onclick = () => {
    void restartGateway();
  };
  el.settingsOnboarding.onclick = () => {
    el.settings.close();
    void state.onboarding!.maybeOpen(true);
  };
  el.settingsShortcuts.onclick = () => {
    el.settings.close();
    openShortcuts();
  };

  el.newSession.onclick = async () => {
    state.current = null;
    el.contextMeter.hidden = true;
    el.dayJump.hidden = true;
    el.chatActions.hidden = true;
    el.chatTitle.textContent = t.session.newTitle;
    el.messages.innerHTML = "";
    renderIntro(el.messages, "new");
    renderSessions();
    el.input.focus();
  };

  // P372: live sidebar session filter.
  el.sessionFilter.addEventListener("input", () => {
    state.sessionFilterText = el.sessionFilter.value;
    renderSessions();
  });

  // P375: day jump scrolls the matching divider into view.
  el.dayJump.addEventListener("change", () => {
    const key = el.dayJump.value;
    const divider = el.messages.querySelector<HTMLElement>(`.day-divider[data-day="${key}"]`);
    divider?.scrollIntoView({ block: "start" });
  });

  // P377: chat-header session actions mirror the sidebar hover set.
  el.chatRename.onclick = () => {
    if (state.current) void renameSession(state.current);
  };
  el.chatExport.onclick = () => {
    if (state.current) void exportSession(state.current, "md");
  };
  el.chatDelete.onclick = () => {
    if (state.current) void deleteSession(state.current);
  };
  el.send.onclick = () => void sendTurn();
  el.mic.onclick = () => void toggleMic();
  el.attachFile.onclick = () => void openFsPicker();
  el.fsUp.onclick = () => {
    const current = el.fsPath.value.trim();
    if (!current) return;
    const parent = current.replace(/\/+$/, "").split("/").slice(0, -1).join("/") || "/";
    void renderFsEntries(parent);
  };
  el.fsGo.onclick = () => {
    const target = el.fsPath.value.trim();
    if (target) void renderFsEntries(target);
  };
  el.fsClose.onclick = () => el.fsDialog.close();
  el.fsMkdir.onclick = () => {
    const name = window.prompt(t.chrome.fsMkdirPrompt);
    if (!name || !name.trim()) return;
    const base = el.fsPath.value.trim().replace(/\/+$/, "");
    const target = `${base}/${name.trim()}`;
    state.client
      ?.fsMkdir(target)
      .then(() => renderFsEntries(base || "/"))
      .catch((error) => {
        el.fsStatus.textContent = fmt(t.chrome.fsFailed, { error });
      });
  };
  el.fsGitRoot.onclick = () => {
    const current = el.fsPath.value.trim();
    if (!current || !state.client) return;
    state.client
      .fsGitRoot(current)
      .then(({ root }) => {
        if (root) return renderFsEntries(root);
        el.fsStatus.textContent = t.chrome.fsGitRootNone;
      })
      .catch((error) => {
        el.fsStatus.textContent = fmt(t.chrome.fsFailed, { error });
      });
  };
  el.fsPreviewSave.onclick = () => void saveFsPreview();
  el.fsPreviewClose.onclick = () => el.fsPreviewDialog.close();
/** P378: terminal-style prompt recall — ↑/↓ walk the sent-prompt
 * history when the slash popup is closed (Up needs the caret at the
 * start so multi-line editing keeps working). */
function handleComposerHistory(event: KeyboardEvent): boolean {
  if (event.shiftKey || event.isComposing) return false;
  const history = state.composerHistory;
  if (event.key === "ArrowUp" && el.input.selectionStart === 0) {
    if (history.length === 0) return false;
    if (state.composerHistoryIndex === null) {
      state.composerDraft = el.input.value;
      state.composerHistoryIndex = history.length - 1;
    } else if (state.composerHistoryIndex > 0) {
      state.composerHistoryIndex -= 1;
    } else {
      return false;
    }
    event.preventDefault();
    el.input.value = history[state.composerHistoryIndex];
    return true;
  }
  if (event.key === "ArrowDown") {
    if (state.composerHistoryIndex === null) return false;
    event.preventDefault();
    if (state.composerHistoryIndex < history.length - 1) {
      state.composerHistoryIndex += 1;
      el.input.value = history[state.composerHistoryIndex];
    } else {
      state.composerHistoryIndex = null;
      el.input.value = state.composerDraft;
    }
    return true;
  }
  return false;
}

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
    // P378: ↑/↓ recall sent prompts (the slash popup owns the arrows
    // while it is open).
    if (handleComposerHistory(event)) return;
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
  el.notifyBell.onclick = () => openNotifyHistory();
  el.chatTitle.addEventListener("click", () => openSessionInfo());
  el.sessionInfoCopy.onclick = () => {
    const id = state.current?.id;
    if (!id) return;
    void navigator.clipboard.writeText(id).then(
      () => notifySuccess(t.session.infoCopied),
      () => notifyError(t.session.infoCopyFailed),
    );
  };
  el.notifyHistoryClear.onclick = () => {
    clearNotificationHistory();
    renderNotifyBadge();
    renderNotifyHistory();
  };
  onNotificationHistoryChange(() => renderNotifyBadge());
  renderNotifyBadge();
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
    void loadAppearance();
  });
  el.settingTheme.addEventListener("change", () => {
    const name = el.settingTheme.value;
    applyTheme(name);
    state.client?.dashboardSetTheme(name).catch(() => undefined);
  });
  el.settingFont.addEventListener("change", () => {
    const font = el.settingFont.value;
    applyFont(font);
    state.client?.dashboardSetFont(font).catch(() => undefined);
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
  const pluginsMain = document.getElementById("plugins")!;
  const pairingMain = document.getElementById("pairing")!;
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
  const tabPlugins = document.getElementById("tab-plugins") as HTMLButtonElement;
  const tabPairing = document.getElementById("tab-pairing") as HTMLButtonElement;
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
  state.pluginsView = new PluginsViewWidget(pluginsMain, () => state.client);
  state.pluginsView.mount();
  state.pairingView = new PairingViewWidget(pairingMain, () => state.client);
  state.pairingView.mount();
  const switchView = (view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing") => {
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
    pluginsMain.hidden = view !== "plugins";
    pairingMain.hidden = view !== "pairing";
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
    tabPlugins.classList.toggle("active", view === "plugins");
    tabPairing.classList.toggle("active", view === "pairing");
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
    if (view === "plugins") {
      state.pluginsView!.start();
    } else {
      state.pluginsView!.stop();
    }
    if (view === "pairing") {
      state.pairingView!.start();
    } else {
      state.pairingView!.stop();
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
  tabPlugins.onclick = () => switchView("plugins");
  tabPairing.onclick = () => switchView("pairing");

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
    search: async (query) => (state.client ? state.client.searchSessions(query, 20) : []),
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
    restartGateway: () => {
      void restartGateway();
    },
    shortcuts: () => openShortcuts(),
    notifications: () => openNotifyHistory(),
    updateCheck: () => runUpdateCheck(),
    kanbanDispatch: () => runKanbanDispatch(),
    kanbanQuickAdd: () => runKanbanQuickAdd(),
  });

  // Translate widget skeletons mounted after the initial applyStatic
  // (idempotent — covers non-en persisted locales on cold boot).
  applyStatic();

  // Locale switch re-render (P251): static attributes are handled by
  // setLocale -> applyStatic; refresh the dynamic chrome and views.
  onLocaleChange(() => {
    renderSessions();
    refreshModelBadge();
    void updateStatusBar();
    if (el.shortcutsDialog.open) renderShortcuts();
    hatchBtn.textContent = t.chrome.hatchPet;
    state.kanban?.rerender();
    state.projects?.rerender();
    state.jobs?.rerender();
    state.findBar?.rerender();
  });

  installHotkeys();
  installDragDrop();
  el.messages.addEventListener("scroll", () => {
    if (chatNearBottom()) el.scrollBottom.hidden = true;
  });
  el.scrollBottom.addEventListener("click", () => {
    el.messages.scrollTop = el.messages.scrollHeight;
    el.scrollBottom.hidden = true;
  });

  await pollHealth();
  el.statusbar.addEventListener("click", () => switchView("doctor"));
  await refreshSessions();
  state.skills = (await state.client.listSkills()) || [];
  setInterval(() => void pollHealth(), 10000);
  setInterval(() => void refreshSessions(), 30000);
  startApprovalWatcher();
}

/** P360: session info popover — clicking the chat title shows the
 * current session's metadata (id, source, model, project, activity,
 * message census) with a copy-id action. */
function formatWhen(ms: number | null | undefined): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function openSessionInfo(): void {
  const session = state.current;
  if (!session) return;
  const rows: [string, string][] = [
    [t.session.infoId, session.id],
    [t.session.infoSource, session.source],
    [t.session.infoModel, session.model || gatewayModel || "—"],
    [t.session.infoProject, session.project || "—"],
    [t.session.infoStarted, formatWhen(session.started_at)],
    [t.session.infoActivity, formatWhen(session.last_activity_at)],
    [t.session.infoMessages, String(session.message_count ?? "—")],
  ];
  el.sessionInfoRows.innerHTML = rows
    .map(([label, value]) => `<div class="notify-history-row"><span class="monitoring-label">${label}</span><span>${value}</span></div>`)
    .join("");
  el.sessionInfoDialog.showModal();
}

function formatTokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1000) return `${(count / 1000).toFixed(1)}k`;
  return String(count);
}

/** P359: chat-header context meter — the current session's total
 * tokens against the gateway model's effective context window. */
async function updateContextMeter(): Promise<void> {
  if (!state.client || !state.current) {
    el.contextMeter.hidden = true;
    return;
  }
  try {
    const [usage, info] = await Promise.all([
      state.client.usage(100),
      state.client.modelInfo(),
    ]);
    const row = usage.sessions.find((session) => session.id === state.current!.id);
    const windowSize = info.context?.effective || 0;
    if (!row || windowSize <= 0) {
      el.contextMeter.hidden = true;
      return;
    }
    const pct = Math.min(100, Math.round((row.total_tokens / windowSize) * 100));
    el.contextMeterFill.style.width = `${pct}%`;
    el.contextMeterText.textContent = `${pct}%`;
    el.contextMeter.title = `${formatTokens(row.total_tokens)} / ${formatTokens(windowSize)} tokens`;
    el.contextMeter.classList.toggle("hot", pct >= 80);
    el.contextMeter.hidden = false;
  } catch {
    el.contextMeter.hidden = true;
  }
}

// ---------------------------------------------------------------------------
// Notification history drawer (P357) — a bell in the sidebar footer with
// an unread badge opens the ring buffer of everything the stacks showed.
// ---------------------------------------------------------------------------

const NOTIFY_KIND_ICON: Record<string, string> = {
  error: "⛔",
  warning: "⚠️",
  info: "ℹ️",
  success: "✅",
};

function renderNotifyBadge(): void {
  const unread = notificationUnread();
  el.notifyBadge.hidden = unread === 0;
  el.notifyBadge.textContent = unread > 99 ? "99+" : String(unread);
}

function notifyTimeAgo(ms: number): string {
  const secs = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function renderNotifyHistory(): void {
  const entries = notificationHistory();
  if (entries.length === 0) {
    el.notifyHistoryList.innerHTML = `<p class="empty">${t.notify.historyEmpty}</p>`;
    return;
  }
  el.notifyHistoryList.innerHTML = entries
    .map((entry) => {
      const icon = NOTIFY_KIND_ICON[entry.kind] ?? NOTIFY_KIND_ICON.info;
      const title = entry.title ? `<strong>${entry.title}</strong> ` : "";
      return `
        <div class="notify-history-row">
          <span>${icon}</span>
          <span>${title}${entry.message}</span>
          <span class="notify-history-time">${notifyTimeAgo(entry.createdAt)}</span>
        </div>`;
    })
    .join("");
}

function openNotifyHistory(): void {
  markNotificationsRead();
  renderNotifyBadge();
  renderNotifyHistory();
  el.notifyHistoryDialog.showModal();
}

// ---------------------------------------------------------------------------
// Keyboard shortcuts cheatsheet (P356) — the hermes-subset chord table
// rendered from i18n, opened from the settings dialog or F1.
// ---------------------------------------------------------------------------

function shortcutMod(): string {
  return navigator.platform.toLowerCase().includes("mac") ? "⌘" : "Ctrl";
}

function shortcutRows(): [string, string][] {
  const mod = shortcutMod();
  return [
    [`${mod}+Shift+M`, t.chrome.scModelPicker],
    [`${mod}+N / Shift+N`, t.chrome.scNewSession],
    ["Ctrl+Tab / Ctrl+Shift+Tab", t.chrome.scCycle],
    [`${mod}+Shift+F`, t.chrome.scSessionPicker],
    [`${mod}+,`, t.chrome.scSettings],
    [`${mod}+B`, t.chrome.scSidebar],
    [`${mod}+K`, t.chrome.scPalette],
    [`${mod}+F`, t.chrome.scFind],
    ["Enter", t.chrome.scFocus],
    ["F1", t.chrome.scShortcuts],
  ];
}

function renderShortcuts(): void {
  el.shortcutsTable.innerHTML = shortcutRows()
    .map(([chord, label]) => `<tr><td>${chord}</td><td>${label}</td></tr>`)
    .join("");
}

function openShortcuts(): void {
  renderShortcuts();
  el.shortcutsDialog.showModal();
}

/** P376: command-palette kanban quick-add — prompt for a title and
 * create the card via POST /api/kanban/tasks. */
async function runKanbanQuickAdd(): Promise<void> {
  if (!state.client) return;
  const title = window.prompt(t.palette.kanbanQuickAddPrompt);
  if (!title || !title.trim()) return;
  try {
    const task = await state.client.kanbanCreateTask(title.trim());
    if (!task) throw new Error("empty task response");
    notifySuccess(fmt(t.palette.kanbanQuickAdded, { id: task.id, title: task.title }));
    state.kanban?.rerender();
  } catch (error) {
    notifyError(fmt(t.palette.kanbanQuickAddFailed, { error: String(error) }));
  }
}

/** P362: command-palette update check — probe /api/update/check and
 * surface the verdict as a toast. */
async function runUpdateCheck(): Promise<void> {
  if (!state.client) return;
  try {
    const check = await state.client.updateCheck();
    let text: string;
    if (check.error) text = check.error;
    else if (!check.update_available) text = t.config.updateUpToDate;
    else if (check.behind === -1) text = t.config.updateShallow;
    else text = t.config.updateBehind.replace("{count}", String(check.behind ?? "?"));
    notifySuccess(`${t.palette.updateCheck}: ${text}`);
  } catch (error) {
    notifyError(
      t.config.updateFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      ),
    );
  }
}

/** P371: command-palette kanban dispatch — POST /api/kanban/dispatch
 * and report spawn/promote/reclaim counts as a toast. */
async function runKanbanDispatch(): Promise<void> {
  if (!state.client) return;
  try {
    const result = await state.client.kanbanDispatch(false);
    if (!result) throw new Error("empty dispatch result");
    notifySuccess(
      fmt(t.palette.kanbanDispatched, {
        spawned: result.spawned,
        promoted: result.promoted,
        reclaimed: result.reclaimed,
      }),
    );
    state.kanban?.rerender();
  } catch (error) {
    notifyError(fmt(t.palette.kanbanDispatchFailed, { error: String(error) }));
  }
}

// Stop a managed gateway when the window closes.
window.addEventListener("beforeunload", () => {
  if (state.bridge && state.managedPid !== null) {
    void state.bridge.stopGateway(state.managedPid);
  }
});

void start();
