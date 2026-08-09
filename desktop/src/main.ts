// ulnclaw desktop UI — session browser + streaming chat against the
// ulnclaw gateway. Tauri commands manage the gateway child process;
// everything else is plain HTTP (gateway.ts).

import { GatewayClient, loadSettings, saveSettings } from "./gateway";
import type { ContextBreakdown, DashboardTheme, FsEntry, GatewaySettings, Project, RetitleSkillsReport, SessionRow, SkillRow, ToolCardEvent } from "./gateway";
import type { KanbanBoard } from "./gateway";
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
import { ProfilesViewWidget } from "./profiles-view";
import { HatchOverlay } from "./hatch";
import { PetOverlay } from "./pet";
import { ModelPickerOverlay } from "./model-picker";
import { FindBar } from "./find-bar";
import { CommandPalette } from "./command-palette";
import { QuickEntry } from "./quick-entry";
import { ArtifactsOverlay } from "./artifacts";
import { LearningOverlay } from "./learning";
import { FileTreePanel } from "./file-tree";
import { notify, notifyError, notifySuccess, notificationHistory, notificationUnread, markNotificationsRead, clearNotificationHistory, onNotificationHistoryChange, loadPersistedNotificationHistory } from "./notifications";
import { hideConnecting, showConnecting } from "./connecting";
import { resolveBootFailure, showBootFailure } from "./boot-failure";
import { applyStatic, currentLocale, fmt, onLocaleChange, t } from "./i18n";
import { LanguageSwitcher } from "./language-switcher";
import { OnboardingOverlay } from "./onboarding";
import { SessionPickerDialog } from "./session-picker";
import { clearIntro, renderIntro } from "./intro";
import { ActivityTimer, formatElapsed } from "./activity-timer";
import { initWakeIndicator } from "./wake";

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

/** P540: native app-menu events (Tauri only) — File › New Session
 * drives the same flow as the sidebar button. */
async function listenMenuEvents(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen("ulnclaw://menu-new-session", () => {
      el.newSession.click();
      el.input.focus();
    });
  } catch {
    /* plain browser tab — no native menu */
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
  /** P615: gateway's persisted reasoning-effort pin (null = endpoint default). */
  reasoningEffort: null as string | null,
  /** P624: last fetched context breakdown for the header popup. */
  contextBreakdown: null as ContextBreakdown | null,
  /** P626: Priority Processing chip state. */
  fastMode: "normal" as string,
  fastSupported: false,
  fastLoaded: false,
  /** P635: approvals-mode chip state. */
  approvalsMode: "manual" as string,
  approvalsModes: [] as string[],
  approvalsLoaded: false,
  reasoningLevels: [] as string[],
  reasoningLoaded: false,
  /** P620: active personality name + configured personas. */
  personalityActive: null as string | null,
  personalityNames: [] as string[],
  personalityLoaded: false,
  /** P386: sidebar session-list cap (raised by the Show all row). */
  sessionListLimit: 100,
  /** P393: per-session composer drafts (session id -> unsent text). */
  drafts: new Map<string, string>(),
  /** P407: sidebar session sort order (persisted). */
  sessionSortMode: (localStorage.getItem("ulnclaw.sessionSort") === "title"
    ? "title"
    : "activity") as "activity" | "title",
  /** P414: hide archived sessions from the sidebar (persisted, on by default). */
  hideArchived: localStorage.getItem("ulnclaw.hideArchived") !== "0",
  /** P532: show only unread sessions in the sidebar (persisted, off by default). */
  unreadOnly: localStorage.getItem("ulnclaw.unreadOnly") === "1",
  /** P394: keyboard navigation over the visible sidebar rows. */
  sessionListVisible: [] as SessionRow[],
  sessionCursor: 0,
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
  profilesView: null as ProfilesViewWidget | null,
  pet: null as PetOverlay | null,
  hatch: null as HatchOverlay | null,
  picker: null as ModelPickerOverlay | null,
  findBar: null as FindBar | null,
  palette: null as CommandPalette | null,
  quickEntry: null as QuickEntry | null,
  artifacts: null as ArtifactsOverlay | null,
  learning: null as LearningOverlay | null,
  fileTree: null as FileTreePanel | null,
  onboarding: null as OnboardingOverlay | null,
  sessionPicker: null as SessionPickerDialog | null,
  view: "chat" as "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing" | "profiles",
};

/** P594: attach a file-tree file to the composer (data-URL round-trip like the fs picker). */
async function attachFsTreeFile(path: string, name: string): Promise<void> {
  if (!state.client) return;
  try {
    const dataUrl = await state.client.fsReadDataUrl(path);
    const response = await fetch(dataUrl);
    const blob = await response.blob();
    const file = new File([blob], name, { type: blob.type || "application/octet-stream" });
    const upload = await state.client.uploadFile(file, name);
    state.pendingUploads.push(upload);
    renderAttachChips();
  } catch (error) {
    addMessage("system", fmt(t.session.uploadFailed, { error }));
  }
}

/** P590: toggle the chat file-tree sidebar, rooted at the open session's cwd. */
async function toggleFileTree(): Promise<void> {
  const panel = state.fileTree;
  if (!panel) return;
  if (panel.visible) {
    panel.hide();
    return;
  }
  let cwd: string | null = null;
  const currentId = state.current?.id ?? null;
  if (currentId && state.client) {
    try {
      const row = await state.client.getSession(currentId);
      cwd = row?.cwd ?? null;
    } catch {
      cwd = null;
    }
  }
  await panel.toggle(cwd);
}

const el = {
  dot: document.getElementById("gateway-dot")!,
  statusbar: document.getElementById("statusbar")!,
  sessionList: document.getElementById("session-list")!,
  sessionFilter: document.getElementById("session-filter") as HTMLInputElement,
  sessionSort: document.getElementById("session-sort") as HTMLButtonElement,
  sessionArchivedToggle: document.getElementById("session-archived") as HTMLButtonElement,
  newSession: document.getElementById("new-session") as HTMLButtonElement,
  messages: document.getElementById("messages")!,
  scrollBottom: document.getElementById("scroll-bottom") as HTMLButtonElement,
  chatTitle: document.getElementById("chat-title")!,
  chatBusy: document.getElementById("chat-busy")!,
  sessionInfoDialog: document.getElementById("session-info-dialog") as HTMLDialogElement,
  sessionInfoRows: document.getElementById("session-info-rows")!,
  sessionInfoCopy: document.getElementById("session-info-copy") as HTMLButtonElement,
  modelBadge: document.getElementById("model-badge")!,
  tokenBadge: document.getElementById("token-badge")!,
  reasoningBadge: document.getElementById("reasoning-badge") as HTMLButtonElement,
  reasoningPop: document.getElementById("reasoning-pop")!,
  personalityBadge: document.getElementById("personality-badge") as HTMLButtonElement,
  personalityPop: document.getElementById("personality-pop")!,
  projectBadge: document.getElementById("project-badge")!,
  endBadge: document.getElementById("end-badge")!,
  contextMeter: document.getElementById("context-meter")!,
  contextMeterFill: document.getElementById("context-meter-fill")!,
  contextMeterText: document.getElementById("context-meter-text")!,
  contextPop: document.getElementById("context-pop")!,
  fastBadge: document.getElementById("fast-badge")!,
  fastPop: document.getElementById("fast-pop")!,
  approvalsBadge: document.getElementById("approvals-badge")!,
  approvalsPop: document.getElementById("approvals-pop")!,
  dayJump: document.getElementById("day-jump") as HTMLSelectElement,
  chatActions: document.getElementById("chat-header-actions")!,
  chatRename: document.getElementById("chat-rename") as HTMLButtonElement,
  chatFork: document.getElementById("chat-fork") as HTMLButtonElement,
  chatArchive: document.getElementById("chat-archive") as HTMLButtonElement,
  chatExport: document.getElementById("chat-export") as HTMLButtonElement,
  chatDelete: document.getElementById("chat-delete") as HTMLButtonElement,
  input: document.getElementById("input") as HTMLTextAreaElement,
  send: document.getElementById("send") as HTMLButtonElement,
  charCount: document.getElementById("char-count")!,
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
  settingReopen: document.getElementById("setting-reopen") as HTMLInputElement,
  settingCharWarn: document.getElementById("setting-char-warn") as HTMLInputElement,
  settingCharLimit: document.getElementById("setting-char-limit") as HTMLInputElement,
  settingNotifySystem: document.getElementById("setting-notify-system") as HTMLInputElement,
  settingTheme: document.getElementById("setting-theme") as HTMLSelectElement,
  settingFont: document.getElementById("setting-font") as HTMLSelectElement,
  settingPersonalityRow: document.getElementById("setting-personality-row") as HTMLLabelElement,
  settingPersonality: document.getElementById("setting-personality") as HTMLSelectElement,
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

/** P384: compact glyphs for well-known non-gateway session sources. */
const SOURCE_BADGES: Record<string, string> = {
  cli: "⌨",
  tui: "⌨",
  import: "⭳",
  "cron-run": "⏰",
  "gateway-run": "⚙",
};

/** P409: short glyphs for session end reasons (complete, branched…). */
const END_BADGES: Record<string, string> = {
  complete: "✓",
  completed: "✓",
  max_iterations: "∞",
  compression: "⧉",
  branched: "⑂",
  ended: "■",
  archived: "🗄",
};

/** P392: sidebar session rows group by relative age. */
function ageGroup(timestamp: number): "today" | "yesterday" | "week" | "older" {
  const now = new Date();
  const date = new Date(timestamp * 1000);
  if (sameCalendarDay(date, now)) return "today";
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameCalendarDay(date, yesterday)) return "yesterday";
  if (now.getTime() - timestamp * 1000 < 7 * 86_400_000) return "week";
  return "older";
}

function ageGroupLabel(group: "today" | "yesterday" | "week" | "older"): string {
  switch (group) {
    case "today":
      return t.session.dayToday;
    case "yesterday":
      return t.session.groupYesterday;
    case "week":
      return t.session.groupWeek;
    default:
      return t.session.groupOlder;
  }
}

/** P407: reflect the session sort mode on the toggle button. */
function refreshSortButton(): void {
  el.sessionSort.textContent = state.sessionSortMode === "title" ? "AZ" : "⏱";
  el.sessionSort.title =
    state.sessionSortMode === "title" ? t.session.sortByTitle : t.session.sortByActivity;
}

/** P414: reflect the hide-archived posture on the toolbar toggle. */
function refreshArchivedButton(): void {
  el.sessionArchivedToggle.classList.toggle("toolbar-on", state.hideArchived);
  el.sessionArchivedToggle.title = state.hideArchived
    ? t.session.showArchived
    : t.session.hideArchived;
}

/** P424: flip the hide-archived sidebar filter (toolbar + palette). */
function toggleHideArchived(): void {
  state.hideArchived = !state.hideArchived;
  try {
    localStorage.setItem(HIDE_ARCHIVED_KEY, state.hideArchived ? "1" : "0");
  } catch {
    /* storage unavailable */
  }
  refreshArchivedButton();
  renderSessions();
}

/** P532: flip the unread-only sidebar filter (palette action). */
function toggleUnreadOnly(): void {
  state.unreadOnly = !state.unreadOnly;
  try {
    localStorage.setItem("ulnclaw.unreadOnly", state.unreadOnly ? "1" : "0");
  } catch {
    /* storage unavailable */
  }
  renderSessions();
  renderUnreadBadge();
}

function renderSessions(): void {
  el.sessionList.innerHTML = "";
  // P372: optional sidebar filter (title or id substring, case-insensitive).
  const filter = state.sessionFilterText.trim().toLowerCase();
  const sorted = [...state.sessions]
    // P407: sort by recent activity or title (locale-aware).
    .sort((a, b) =>
      state.sessionSortMode === "title"
        ? (a.title || a.id.slice(0, 8)).localeCompare(b.title || b.id.slice(0, 8))
        : b.last_activity_at - a.last_activity_at,
    )
    // P414: archived sessions stay out of the sidebar unless toggled in.
    .filter((session) => !state.hideArchived || session.end_reason !== "archived")
    // P532: optional unread-only view for triaging fresh replies.
    .filter((session) => !state.unreadOnly || unreadSessions.has(session.id))
    .filter((session) => {
      if (!filter) return true;
      const title = session.title || session.id.slice(0, 8);
      return (
        title.toLowerCase().includes(filter) ||
        session.id.toLowerCase().includes(filter)
      );
    });
  // P548: pinned sessions always float to the top of the list.
  const pinnedRows = sorted.filter((session) => pinnedSessions.has(session.id));
  const unpinnedRows = sorted.filter((session) => !pinnedSessions.has(session.id));
  const visible = [...pinnedRows, ...unpinnedRows].slice(0, state.sessionListLimit);
  state.sessionListVisible = visible;
  state.sessionCursor = Math.min(state.sessionCursor, Math.max(0, visible.length - 1));
  let lastGroup: "pinned" | "today" | "yesterday" | "week" | "older" | "" = "";
  let rowIndex = 0;
  for (const session of visible) {
    // P548: pinned rows sit under a single 📌 header; the rest keep
    // the P392 relative-date group headers.
    if (pinnedSessions.has(session.id)) {
      if (lastGroup !== "pinned") {
        const header = document.createElement("div");
        header.className = "session-group-header";
        header.textContent = `\u{1F4CC} ${t.session.pinnedGroup}`;
        el.sessionList.appendChild(header);
        lastGroup = "pinned";
      }
    } else {
      const group = ageGroup(session.last_activity_at);
      if (group !== lastGroup) {
        const header = document.createElement("div");
        header.className = "session-group-header";
        header.textContent = ageGroupLabel(group);
        el.sessionList.appendChild(header);
        lastGroup = group;
      }
    }
    const item = document.createElement("div");
    item.className =
      "session-item" +
      (state.current?.id === session.id ? " active" : "") +
      (rowIndex === state.sessionCursor ? " cursor" : "");
    rowIndex += 1;
    const title = session.title || session.id.slice(0, 8);
    const whenFull = new Date(session.last_activity_at * 1000).toLocaleString();
    const main = document.createElement("div");
    main.className = "session-main";
    main.innerHTML = `<span class="title"></span><span class="when"></span>`;
    main.querySelector(".title")!.textContent = pinnedSessions.has(session.id)
      ? `\u{1F4CC} ${title}`
      : title;
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
    // P380: per-session model badge in the sidebar row.
    if (session.model) {
      whenEl.textContent += ` · ${session.model}`;
    }
    // P628: per-row context-window usage (hermes session-picker
    // context_pct parity); hot styling at 80%+.
    if (typeof session.context_percent === "number" && session.context_percent > 0) {
      const ctxEl = document.createElement("span");
      ctxEl.className = "session-ctx-badge" + (session.context_percent >= 80 ? " hot" : "");
      ctxEl.title = fmt(t.session.ctxTitle, { pct: String(session.context_percent) });
      ctxEl.textContent = `${session.context_percent}% ctx`;
      whenEl.appendChild(ctxEl);
    }
    item.appendChild(main);
    if (session.project) {
      const badge = document.createElement("span");
      badge.className = "session-project-badge";
      badge.title = fmt(t.session.projectBadge, { project: session.project });
      badge.textContent = session.project;
      item.appendChild(badge);
    }
    // P384: badge non-gateway origins (cli/import/cron…) at a glance.
    if (session.source && session.source !== "gateway") {
      const sourceBadge = document.createElement("span");
      sourceBadge.className = "session-source-badge";
      sourceBadge.title = session.source;
      sourceBadge.textContent = SOURCE_BADGES[session.source] ?? session.source;
      item.appendChild(sourceBadge);
    }
    // P409: badge sessions that carry an end reason.
    if (session.end_reason) {
      const endBadge = document.createElement("span");
      endBadge.className = "session-end-badge";
      endBadge.title = fmt(t.session.endReasonTitle, { reason: session.end_reason });
      endBadge.textContent = END_BADGES[session.end_reason] ?? session.end_reason;
      item.appendChild(endBadge);
    }
    const actions = document.createElement("span");
    actions.className = "session-actions";
    // P548: pin/unpin floats a session to the top of the list.
    const pinBtn = document.createElement("button");
    pinBtn.className = "icon";
    const rowPinned = pinnedSessions.has(session.id);
    pinBtn.title = rowPinned ? t.palette.unpinSession : t.palette.pinSession;
    pinBtn.textContent = rowPinned ? "\u{1F4CC}" : "\u{1F4CD}";
    pinBtn.onclick = (event) => {
      event.stopPropagation();
      togglePinSession(session);
    };
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
    // P541: archive/unarchive joins the sidebar hover action set.
    const archiveBtn = document.createElement("button");
    archiveBtn.className = "icon";
    const rowArchived = session.end_reason === "archived";
    archiveBtn.title = rowArchived ? t.palette.unarchiveSession : t.palette.archiveSession;
    archiveBtn.textContent = rowArchived ? "♻" : "🗄";
    archiveBtn.onclick = (event) => {
      event.stopPropagation();
      void toggleSessionArchived(session);
    };
    const deleteBtn = document.createElement("button");
    deleteBtn.className = "icon danger";
    deleteBtn.title = t.palette.deleteSession;
    deleteBtn.textContent = "🗑";
    deleteBtn.onclick = (event) => {
      event.stopPropagation();
      void deleteSession(session);
    };
    actions.append(pinBtn, renameBtn, exportBtn, archiveBtn, deleteBtn);
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
  // P386: the list is capped; offer an expander when rows overflow.
  if (sorted.length > state.sessionListLimit) {
    const more = document.createElement("button");
    more.className = "ghost session-show-all";
    more.textContent = fmt(t.session.showAll, { count: sorted.length });
    more.onclick = () => {
      state.sessionListLimit = sorted.length;
      renderSessions();
    };
    el.sessionList.appendChild(more);
  } else if (state.sessionListLimit > 100 && sorted.length > 100) {
    const less = document.createElement("button");
    less.className = "ghost session-show-all";
    less.textContent = t.session.showLess;
    less.onclick = () => {
      state.sessionListLimit = 100;
      renderSessions();
    };
    el.sessionList.appendChild(less);
  }
}

/** P543: double-click the chat title to rename inline — Enter saves,
 * Esc cancels, blur commits. The prompt-based flow stays on the ✎
 * button; a single click still opens the session-info popover. */
function beginInlineRename(): void {
  if (!state.current || !state.client) return;
  const header = el.chatTitle.parentElement;
  if (!header || header.querySelector("#chat-title-inline")) return;
  const session = state.current;
  const current = session.title || session.id.slice(0, 8);
  const input = document.createElement("input");
  input.id = "chat-title-inline";
  input.type = "text";
  input.className = "chat-title-inline";
  input.maxLength = 100;
  input.value = current;
  el.chatTitle.hidden = true;
  header.insertBefore(input, el.chatTitle.nextSibling);
  input.focus();
  input.select();
  let done = false;
  const finish = (commit: boolean): void => {
    if (done) return;
    done = true;
    const value = input.value.trim();
    input.remove();
    el.chatTitle.hidden = false;
    if (commit && value && value !== current) {
      void applyInlineRename(session, value);
    }
  };
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      finish(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      finish(false);
    }
  });
  input.addEventListener("blur", () => finish(true));
}

/** P543: persist the inline-rename result (shared success/failure
 * toasts with the prompt flow). */
async function applyInlineRename(session: SessionRow, title: string): Promise<void> {
  if (!state.client) return;
  try {
    await state.client.renameSession(session.id, title);
    session.title = title;
    if (state.current?.id === session.id) {
      state.current.title = title;
      el.chatTitle.textContent = title;
      refreshWindowTitle();
    }
    renderSessions();
    notifySuccess(t.session.renamed);
  } catch (error) {
    notifyError(fmt(t.session.renameFailed, { error }));
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
      refreshWindowTitle();
    }
    renderSessions();
    notifySuccess(t.session.renamed);
  } catch (error) {
    notifyError(fmt(t.session.renameFailed, { error }));
  }
}

/** P560: regenerate the current session's title via the LLM titler
 * (gateway P559) and sync the header, sidebar and window title. */
async function runRetitleSession(): Promise<void> {
  const session = state.current;
  if (!session || !state.client) return;
  try {
    const result = await state.client.retitleSession(session.id, true);
    if (!result || result.status === "rejected") {
      notifyError(t.palette.retitleCurrentFailed);
      return;
    }
    if (result.status === "unchanged") {
      notifySuccess(t.palette.retitleCurrentUnchanged);
      return;
    }
    session.title = result.new_title;
    state.current!.title = result.new_title;
    el.chatTitle.textContent = result.new_title;
    refreshWindowTitle();
    renderSessions();
    notifySuccess(fmt(t.palette.retitledCurrent, { label: result.new_title }));
  } catch {
    notifyError(t.palette.retitleCurrentFailed);
  }
}

async function exportSession(session: SessionRow, format: "md" | "html" | "json"): Promise<void> {
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

/** P415: chat-header export picker — Markdown / HTML / portable JSON. */
function openExportPicker(): void {
  if (!state.current) return;
  const session = state.current;
  const dialog = document.createElement("dialog");
  dialog.className = "theme-picker-dialog";
  const heading = document.createElement("div");
  heading.className = "theme-picker-title";
  heading.textContent = t.palette.exportPickerTitle;
  const list = document.createElement("div");
  list.className = "theme-picker-list";
  const options: { format: "md" | "html" | "json"; label: string }[] = [
    { format: "md", label: t.sessionsView.exportTitle },
    { format: "html", label: t.sessionsView.exportHtmlTitle },
    { format: "json", label: t.sessionsView.exportJsonTitle },
  ];
  for (const option of options) {
    const row = document.createElement("button");
    row.className = "theme-picker-item";
    row.textContent = option.label;
    row.onclick = () => {
      dialog.close();
      void exportSession(session, option.format);
    };
    list.appendChild(row);
  }
  dialog.append(heading, list);
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.appendChild(dialog);
  dialog.showModal();
}

// P473: ids this shell deleted itself (suppresses cross-client toasts).
const localDeletes = new Set<string>();

// P494: debounce timer for list refreshes driven by session.message.
let sessionMessageListTimer: number | null = null;

// P495: debounce timer for the chat view's live catch-up.
let chatCatchupTimer: number | null = null;

// P496/P500: sessions that grew while not open (cleared on open/select),
// persisted across restarts.
const UNREAD_KEY = "ulnclaw.unreadSessions";
const unreadSessions = new Set<string>((() => {
  try {
    const raw = localStorage.getItem(UNREAD_KEY);
    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.filter((id): id is string => typeof id === "string");
    }
  } catch {
    /* storage unavailable */
  }
  return [];
})());

function persistUnread(): void {
  try {
    localStorage.setItem(UNREAD_KEY, JSON.stringify([...unreadSessions]));
  } catch {
    /* storage unavailable */
  }
}

/** P548: pinned sessions float to the top of the sidebar (persisted). */
const PINNED_KEY = "ulnclaw.pinnedSessions";
const pinnedSessions = new Set<string>((() => {
  try {
    const raw = localStorage.getItem(PINNED_KEY);
    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.filter((id): id is string => typeof id === "string");
    }
  } catch {
    /* storage unavailable */
  }
  return [];
})());

function persistPinned(): void {
  try {
    localStorage.setItem(PINNED_KEY, JSON.stringify([...pinnedSessions]));
  } catch {
    /* storage unavailable */
  }
}

/** P548: toggle a session's pin (sidebar hover action + palette). */
function togglePinSession(session: SessionRow): void {
  const label = session.title || session.id.slice(0, 8);
  if (pinnedSessions.has(session.id)) {
    pinnedSessions.delete(session.id);
    persistPinned();
    renderSessions();
    notifySuccess(fmt(t.palette.sessionUnpinned, { label }));
  } else {
    pinnedSessions.add(session.id);
    persistPinned();
    renderSessions();
    notifySuccess(fmt(t.palette.sessionPinned, { label }));
  }
}

/** P497: badge the Sessions tab with the unread count. */
function renderUnreadBadge(): void {
  const tab = document.getElementById("tab-sessions-view");
  if (!tab) return;
  const count = unreadSessions.size;
  let badge = tab.querySelector<HTMLSpanElement>(".tab-badge");
  if (count === 0) {
    badge?.remove();
    tab.title = "";
    return;
  }
  if (!badge) {
    badge = document.createElement("span");
    badge.className = "tab-badge";
    tab.appendChild(badge);
  }
  badge.textContent = String(count);
  tab.title = fmt(t.session.unreadTitle, { count: String(count) });
}

async function deleteSession(session: SessionRow): Promise<void> {
  if (!state.client) return;
  const label = session.title || session.id.slice(0, 8);
  if (!window.confirm(fmt(t.session.deleteConfirm, { label }))) return;
  try {
    await state.client.deleteSession(session.id);
    localDeletes.add(session.id);
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

function addMessage(role: string, content: string, timestamp?: number): HTMLElement {
  const row = document.createElement("div");
  row.className = `message ${role}`;
  // P381: hovering a bubble shows the stored message time.
  if (typeof timestamp === "number" && timestamp > 0) {
    row.title = new Date(timestamp * 1000).toLocaleString();
  }
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

/** P387: window title tracks the current session (webview title bar). */
function refreshWindowTitle(): void {
  const title = state.current
    ? state.current.title || state.current.id.slice(0, 8)
    : null;
  const base = title ? `${title} — ulnclaw` : "ulnclaw";
  // P404: unread notification count prefixes the title.
  const unread = notificationUnread();
  document.title = unread > 0 ? `(${unread > 99 ? "99+" : unread}) ${base}` : base;
}

async function openSession(session: SessionRow): Promise<void> {
  // P496: opening a session reads it.
  if (unreadSessions.delete(session.id)) {
    persistUnread();
    renderSessions();
    renderUnreadBadge();
  }
  // P393: stash the outgoing session's draft, restore the target's.
  if (state.current && state.current.id !== session.id) {
    state.drafts.set(state.current.id, el.input.value);
    persistDrafts();
  }
  state.current = session;
  // P595: the file tree follows the session's working folder while visible.
  if (state.fileTree?.visible) {
    void state.fileTree.open(session.cwd ?? null);
  }
  el.chatTitle.textContent = session.title || session.id.slice(0, 8);
  refreshWindowTitle();
  refreshModelBadge();
  try {
    // P399: remember the last open session for the next launch.
    localStorage.setItem(LAST_SESSION_KEY, session.id);
  } catch {
    /* storage unavailable */
  }
  el.input.value = state.drafts.get(session.id) ?? "";
  refreshCharCount();
  hideSlashPop();
  el.messages.innerHTML = "";
  el.dayJump.hidden = true;
  el.chatActions.hidden = false;
  renderSessions();
  // P417: bring the freshly activated row into view (palette/picker
  // jumps may land far from the current scroll position).
  el.sessionList
    .querySelector(".session-item.active")
    ?.scrollIntoView({ block: "nearest" });
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
      addMessage(message.role, message.content, message.timestamp);
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

/** P402: Alt+↑/Alt+↓ jumps between transcript day dividers. */
function jumpDayDivider(direction: -1 | 1): void {
  const dividers = [...el.messages.querySelectorAll<HTMLElement>(".day-divider")];
  if (dividers.length === 0) return;
  const containerTop = el.messages.getBoundingClientRect().top;
  const position = (node: HTMLElement) =>
    node.getBoundingClientRect().top - containerTop + el.messages.scrollTop;
  const top = el.messages.scrollTop;
  const target =
    direction === 1
      ? dividers.find((divider) => position(divider) > top + 8)
      : [...dividers].reverse().find((divider) => position(divider) < top - 8);
  target?.scrollIntoView({ block: "start" });
}

/** P388: sidebar collapse toggling, persisted across launches. */
const SIDEBAR_COLLAPSED_KEY = "ulnclaw.sidebarCollapsed";

/** P395: composer drafts persist across restarts (session id -> text). */
const DRAFTS_KEY = "ulnclaw.drafts";

/** P396: ↑/↓ prompt-recall history persists across restarts. */
const COMPOSER_HISTORY_KEY = "ulnclaw.composerHistory";

/** P397: sidebar session filter persists across restarts. */
const SESSION_FILTER_KEY = "ulnclaw.sessionFilter";

/** P407: session sort order persists across restarts. */
const SESSION_SORT_KEY = "ulnclaw.sessionSort";

/** P414: the hide-archived sidebar toggle persists across restarts. */
const HIDE_ARCHIVED_KEY = "ulnclaw.hideArchived";

/** P398: the active view persists across restarts. */
const ACTIVE_VIEW_KEY = "ulnclaw.activeView";

/** P399: the last open session reopens at boot. */
const LAST_SESSION_KEY = "ulnclaw.lastSession";

function persistComposerHistory(): void {
  try {
    localStorage.setItem(COMPOSER_HISTORY_KEY, JSON.stringify(state.composerHistory));
  } catch {
    /* storage full/unavailable — history stays in-memory */
  }
}

function loadPersistedComposerHistory(): void {
  try {
    const raw = localStorage.getItem(COMPOSER_HISTORY_KEY);
    if (!raw) return;
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      state.composerHistory = parsed.filter((v): v is string => typeof v === "string").slice(-50);
    }
  } catch {
    /* corrupt payload — start fresh */
  }
}

function persistDrafts(): void {
  try {
    let entries = [...state.drafts.entries()];
    if (entries.length > 50) entries = entries.slice(entries.length - 50);
    localStorage.setItem(DRAFTS_KEY, JSON.stringify(Object.fromEntries(entries)));
  } catch {
    /* storage full/unavailable — drafts stay in-memory */
  }
}

function loadPersistedDrafts(): void {
  try {
    const raw = localStorage.getItem(DRAFTS_KEY);
    if (!raw) return;
    const parsed: unknown = JSON.parse(raw);
    if (parsed && typeof parsed === "object") {
      for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
        if (typeof value === "string") state.drafts.set(id, value);
      }
    }
  } catch {
    /* corrupt payload — start fresh */
  }
}

function toggleSidebar(): void {
  const app = document.getElementById("app")!;
  app.classList.toggle("no-sidebar");
  localStorage.setItem(
    SIDEBAR_COLLAPSED_KEY,
    app.classList.contains("no-sidebar") ? "1" : "0",
  );
}

async function refreshSessions(): Promise<void> {
  if (!state.client) return;
  try {
    // P565: load last-message previews so the session picker (and any
    // future surface) can show snippets without extra round-trips.
    // P628: context=true adds per-row context_percent for the sidebar.
    state.sessions = await state.client.listSessions(true, true);
    await refreshSessionTokens();
    // P545: usage just landed — refresh the open session's token chip.
    refreshTokenBadge();
    // P500: drop unread markers for sessions that no longer exist.
    const known = new Set(state.sessions.map((session) => session.id));
    let changed = false;
    for (const id of [...unreadSessions]) {
      if (!known.has(id)) {
        unreadSessions.delete(id);
        changed = true;
      }
    }
    if (changed) {
      persistUnread();
      renderUnreadBadge();
    }
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
      refreshCharCount();
      el.input.focus();
    }
  } catch (error) {
    notifyError(fmt(t.chrome.micFailed, { error: String(error) }));
  }
}

/** P391: live character counter for the composer draft. */
function refreshCharCount(): void {
  const length = el.input.value.length;
  el.charCount.textContent = String(length);
  el.charCount.hidden = length === 0;
  // P434: amber from 80% of the configured threshold, red at/above it.
  const threshold = state.settings.charWarn;
  el.charCount.classList.toggle("warn", threshold > 0 && length >= threshold * 0.8 && length < threshold);
  el.charCount.classList.toggle("over", threshold > 0 && length >= threshold);
  // P466: hard-limit state renders distinctly and blocks sending.
  const limit = state.settings.charLimit;
  el.charCount.classList.toggle("blocked", limit > 0 && length > limit);
}

async function sendTurn(): Promise<void> {
  const text = el.input.value.trim();
  // P466: optional hard limit blocks oversized messages before any dispatch.
  const charLimit = state.settings.charLimit;
  if (charLimit > 0 && text.length > charLimit) {
    notifyError(fmt(t.chrome.charLimitExceeded, { length: String(text.length), limit: String(charLimit) }));
    refreshCharCount();
    return;
  }
  // /resume + /sessions + /switch render the desktop session picker
  // (P254, hermes composer parity) instead of hitting the headless
  // gateway slash worker, which can't show the overlay.
  if (/^\/(resume|sessions|switch)\b/i.test(text)) {
    el.input.value = "";
    refreshCharCount();
    state.sessionPicker?.open();
    return;
  }
  // /new starts a fresh session straight from the composer (P374,
  // hermes CLI /new parity).
  if (/^\/new\b/i.test(text)) {
    el.input.value = "";
    refreshCharCount();
    el.newSession.click();
    return;
  }
  // P410: /clear clears the screen and starts a new session (hermes
  // CLI /clear parity — destructive, so it asks first).
  if (/^\/clear\b/i.test(text)) {
    el.input.value = "";
    refreshCharCount();
    if (window.confirm(t.slash.clearConfirm)) el.newSession.click();
    return;
  }
  // P456: view-switching slash commands (desktop-only; no gateway clash).
  const viewMatch = /^\/(doctor|runs|projects|config|webhooks|models|plugins)\b/i.exec(text);
  if (viewMatch) {
    el.input.value = "";
    refreshCharCount();
    const view = viewMatch[1].toLowerCase() as
      | "doctor" | "runs" | "projects" | "config" | "webhooks" | "models" | "plugins";
    activeSwitchView?.(view);
    return;
  }
  // P491: /open <id-prefix> resumes a session by id prefix.
  const openMatch = /^\/open\s+(\S+)\s*$/i.exec(text);
  if (openMatch) {
    el.input.value = "";
    refreshCharCount();
    const prefix = openMatch[1].toLowerCase();
    const match = state.sessions.find((session) => session.id.toLowerCase().startsWith(prefix));
    if (match) {
      void openSession(match);
    } else {
      notifyError(fmt(t.slash.openNotFound, { prefix }));
    }
    return;
  }
  // P490: /search <query> opens the Sessions view with an FTS search.
  const searchMatch = /^\/search\s+(.+)$/i.exec(text);
  if (searchMatch) {
    el.input.value = "";
    refreshCharCount();
    activeSwitchView?.("sessions");
    state.sessionsBrowser?.searchFor(searchMatch[1].trim());
    return;
  }
  // P459: /archive + /unarchive manage the current session's end state.
  if (/^\/archive\b/i.test(text)) {
    el.input.value = "";
    refreshCharCount();
    void runArchiveSession();
    return;
  }
  if (/^\/unarchive\b/i.test(text)) {
    el.input.value = "";
    refreshCharCount();
    void runUnarchiveSession();
    return;
  }
  // P485: /fork branches the current session and jumps into the fork.
  if (/^\/fork\b/i.test(text)) {
    el.input.value = "";
    refreshCharCount();
    void runForkSession();
    return;
  }
  // P484: /export [md|html|json] downloads the current session transcript.
  const exportMatch = /^\/export(?:\s+(md|html|json))?\s*$/i.exec(text);
  if (exportMatch) {
    el.input.value = "";
    refreshCharCount();
    if (state.current) {
      const format = (exportMatch[1] || "md").toLowerCase() as "md" | "html" | "json";
      void exportSession(state.current, format);
    }
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
      try {
        localStorage.setItem(LAST_SESSION_KEY, state.current.id);
      } catch {
        /* storage unavailable */
      }
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
    persistComposerHistory();
  }
  state.composerHistoryIndex = null;
  if (state.current) {
    state.drafts.delete(state.current.id);
    persistDrafts();
  }
  state.busy = true;
  el.send.disabled = true;
  // P390: thinking indicator while the turn streams.
  el.chatBusy.textContent = t.tools.thinking;
  el.chatBusy.hidden = false;
  el.input.value = "";
  refreshCharCount();
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
    el.chatBusy.hidden = true;
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
/** P383: composer placeholder carries the active session's model so
 * the user always knows which model they are talking to. */
/** P634: fill the settings-dialog personality picker from
 * GET /api/personalities; hides the row when nothing is configured. */
async function fillPersonalityPicker(): Promise<void> {
  if (!state.client) return;
  try {
    const payload = await state.client.personalitiesGet();
    const list = payload.personalities ?? [];
    if (list.length === 0) {
      el.settingPersonalityRow.hidden = true;
      return;
    }
    el.settingPersonality.innerHTML = "";
    const def = document.createElement("option");
    def.value = "";
    def.textContent = t.chrome.personalityDefault;
    el.settingPersonality.appendChild(def);
    for (const persona of list) {
      const option = document.createElement("option");
      option.value = persona.name;
      option.textContent = persona.name;
      option.title = persona.preview;
      el.settingPersonality.appendChild(option);
    }
    el.settingPersonality.value = state.personalityActive ?? "";
    if (el.settingPersonality.value !== (state.personalityActive ?? "")) {
      el.settingPersonality.value = "";
    }
    el.settingPersonalityRow.hidden = false;
  } catch {
    el.settingPersonalityRow.hidden = true;
  }
}

/** P634: apply the settings-dialog personality pick (PUT /api/personality)
 * and sync the header chip. */
async function applySettingsPersonality(): Promise<void> {
  if (!state.client || el.settingPersonalityRow.hidden) return;
  const pick = el.settingPersonality.value;
  if (pick === (state.personalityActive ?? "")) return;
  try {
    const payload = await state.client.personalitySet(pick === "" ? null : pick);
    state.personalityActive = payload.active;
    refreshPersonalityBadge();
  } catch (error) {
    notifyError(String(error));
  }
}

function refreshComposerPlaceholder(): void {
  const base = t.chrome.inputPlaceholder;
  const model = state.current?.model;
  el.input.placeholder = model ? `${base} — ${model}` : base;
}

/** P615: reasoning-effort chip beside the model badge — shows the
 * gateway's persisted agent.reasoning_effort pin (⚡ auto when clear);
 * clicking pops the level list and persists the pick. */
function refreshReasoningBadge(): void {
  const effort = state.reasoningEffort;
  el.reasoningBadge.textContent = effort
    ? `\u{26A1} ${effort}`
    : `\u{26A1} ${t.chrome.reasoningAuto}`;
  el.reasoningBadge.title = t.chrome.reasoningTitle;
  el.reasoningBadge.classList.toggle("pinned", !!effort);
  el.reasoningBadge.hidden = false;
}

async function loadReasoningState(): Promise<void> {
  if (!state.client) return;
  try {
    const payload = await state.client.reasoningGet();
    state.reasoningEffort = payload.effort;
    state.reasoningLevels = payload.levels ?? [];
    refreshReasoningBadge();
  } catch {
    // Gateway without /api/reasoning (or unreachable): keep chip hidden.
  }
}

/** P635: approvals-mode chip (hermes approval-mode-menu parity):
 * shows the persisted approvals.mode; the popup switches it through
 * PUT /api/approvals. */
function refreshApprovalsBadge(): void {
  const mode = state.approvalsMode;
  el.approvalsBadge.textContent = `\u{1F6E1} ${mode}`;
  el.approvalsBadge.title = t.chrome.approvalsTitle;
  el.approvalsBadge.classList.toggle("pinned", mode === "off");
  el.approvalsBadge.hidden = false;
}

async function loadApprovalsState(): Promise<void> {
  if (!state.client) return;
  try {
    const payload = await state.client.approvalsGet();
    state.approvalsMode = payload.mode;
    state.approvalsModes = payload.modes ?? [];
    refreshApprovalsBadge();
  } catch {
    // Gateway without /api/approvals (or unreachable): keep chip hidden.
  }
}

function toggleApprovalsPop(): void {
  if (!el.approvalsPop.hidden) {
    el.approvalsPop.hidden = true;
    return;
  }
  const modes = state.approvalsModes.length
    ? state.approvalsModes
    : ["manual", "smart", "off"];
  el.approvalsPop.innerHTML = modes
    .map((mode) => {
      const active = mode === state.approvalsMode;
      return `<div class="slash-item${active ? " selected" : ""}" data-mode="${mode}">\u{1F6E1} ${escapeHtmlInfo(mode)}</div>`;
    })
    .join("");
  const rect = el.approvalsBadge.getBoundingClientRect();
  el.approvalsPop.style.position = "fixed";
  el.approvalsPop.style.left = `${rect.left}px`;
  el.approvalsPop.style.top = `${rect.bottom + 6}px`;
  el.approvalsPop.style.right = "auto";
  el.approvalsPop.style.bottom = "auto";
  el.approvalsPop.style.minWidth = "120px";
  el.approvalsPop.hidden = false;
}

async function applyApprovalsPick(mode: string): Promise<void> {
  el.approvalsPop.hidden = true;
  if (!state.client) return;
  try {
    const payload = await state.client.approvalsSet(mode);
    state.approvalsMode = payload.mode;
    refreshApprovalsBadge();
    notifySuccess(`\u{1F6E1} ${payload.mode}`);
  } catch (error) {
    notifyError(String(error));
  }
}

/** P626: Priority Processing chip (hermes /fast desktop parity) —
 * hidden unless the gateway reports the model supports it. */
function refreshFastBadge(): void {
  if (!state.fastSupported) {
    el.fastBadge.hidden = true;
    return;
  }
  const fast = state.fastMode === "fast";
  el.fastBadge.textContent = fast
    ? `\u{1F680} ${t.chrome.fastOn}`
    : `\u{1F680} ${t.chrome.fastOff}`;
  el.fastBadge.title = t.chrome.fastTitle;
  el.fastBadge.classList.toggle("pinned", fast);
  el.fastBadge.hidden = false;
}

async function loadFastState(): Promise<void> {
  if (!state.client) return;
  try {
    const payload = await state.client.fastGet();
    state.fastSupported = payload.supported;
    state.fastMode = payload.mode;
    refreshFastBadge();
  } catch {
    // Gateway without /api/fast (or unreachable): keep chip hidden.
  }
}

function toggleFastPop(): void {
  if (!el.fastPop.hidden) {
    el.fastPop.hidden = true;
    return;
  }
  const items: Array<{ mode: "fast" | "normal"; label: string }> = [
    { mode: "fast", label: `\u{1F680} ${t.chrome.fastOn}` },
    { mode: "normal", label: `\u{1F680} ${t.chrome.fastOff}` },
  ];
  el.fastPop.innerHTML = items
    .map((item) => {
      const active = item.mode === state.fastMode;
      return `<div class="slash-item${active ? " selected" : ""}" data-mode="${item.mode}">${escapeHtmlInfo(item.label)}</div>`;
    })
    .join("");
  const rect = el.fastBadge.getBoundingClientRect();
  el.fastPop.style.position = "fixed";
  el.fastPop.style.left = `${rect.left}px`;
  el.fastPop.style.top = `${rect.bottom + 6}px`;
  el.fastPop.style.right = "auto";
  el.fastPop.style.bottom = "auto";
  el.fastPop.style.minWidth = "140px";
  el.fastPop.hidden = false;
}

async function applyFastPick(mode: "fast" | "normal"): Promise<void> {
  el.fastPop.hidden = true;
  if (!state.client) return;
  try {
    const payload = await state.client.fastSet(mode);
    state.fastMode = payload.mode;
    refreshFastBadge();
    notifySuccess(
      payload.mode === "fast"
        ? `\u{1F680} ${t.chrome.fastOn}`
        : `\u{1F680} ${t.chrome.fastOff}`,
    );
  } catch (error) {
    notifyError(String(error));
  }
}

function toggleReasoningPop(): void {
  if (!el.reasoningPop.hidden) {
    el.reasoningPop.hidden = true;
    return;
  }
  const items: (string | null)[] = [null, ...state.reasoningLevels];
  el.reasoningPop.innerHTML = items
    .map((level) => {
      const label = level ?? t.chrome.reasoningAuto;
      const active = (level ?? "") === (state.reasoningEffort ?? "");
      return `<div class="slash-item${active ? " selected" : ""}" data-level="${level ?? ""}">\u{26A1} ${escapeHtmlInfo(label)}</div>`;
    })
    .join("");
  const rect = el.reasoningBadge.getBoundingClientRect();
  el.reasoningPop.style.position = "fixed";
  el.reasoningPop.style.left = `${rect.left}px`;
  el.reasoningPop.style.top = `${rect.bottom + 6}px`;
  el.reasoningPop.style.right = "auto";
  el.reasoningPop.style.bottom = "auto";
  el.reasoningPop.style.minWidth = "160px";
  el.reasoningPop.hidden = false;
}

async function applyReasoningPick(level: string | null): Promise<void> {
  el.reasoningPop.hidden = true;
  if (!state.client) return;
  try {
    const payload = await state.client.reasoningSet(level);
    state.reasoningEffort = payload.effort;
    refreshReasoningBadge();
    notifySuccess(
      payload.effort
        ? `\u{26A1} ${payload.effort}`
        : `\u{26A1} ${t.chrome.reasoningAuto}`,
    );
  } catch (error) {
    notifyError(String(error));
  }
}

/** P620: personality chip beside the reasoning badge — shows the
 * active persona (🎭 default when clear); clicking pops the persona
 * list and persists the pick through PUT /api/personality. */
function refreshPersonalityBadge(): void {
  const active = state.personalityActive;
  el.personalityBadge.textContent = active
    ? `\u{1F3AD} ${active}`
    : `\u{1F3AD} ${t.chrome.personalityDefault}`;
  el.personalityBadge.title = t.chrome.personalityTitle;
  el.personalityBadge.classList.toggle("pinned", !!active);
  el.personalityBadge.hidden = false;
}

async function loadPersonalityState(): Promise<void> {
  if (!state.client) return;
  try {
    const payload = await state.client.personalitiesGet();
    state.personalityActive = payload.active;
    state.personalityNames = (payload.personalities ?? []).map((p) => p.name);
    refreshPersonalityBadge();
  } catch {
    // Gateway without /api/personalities (or none configured): hide.
  }
}

function togglePersonalityPop(): void {
  if (!el.personalityPop.hidden) {
    el.personalityPop.hidden = true;
    return;
  }
  const items: (string | null)[] = [null, ...state.personalityNames];
  el.personalityPop.innerHTML = items
    .map((name) => {
      const label = name ?? t.chrome.personalityDefault;
      const active = (name ?? "") === (state.personalityActive ?? "");
      return `<div class="slash-item${active ? " selected" : ""}" data-name="${name ?? ""}">\u{1F3AD} ${escapeHtmlInfo(label)}</div>`;
    })
    .join("");
  const rect = el.personalityBadge.getBoundingClientRect();
  el.personalityPop.style.position = "fixed";
  el.personalityPop.style.left = `${rect.left}px`;
  el.personalityPop.style.top = `${rect.bottom + 6}px`;
  el.personalityPop.style.right = "auto";
  el.personalityPop.style.bottom = "auto";
  el.personalityPop.style.minWidth = "160px";
  el.personalityPop.hidden = false;
}

async function applyPersonalityPick(name: string | null): Promise<void> {
  el.personalityPop.hidden = true;
  if (!state.client) return;
  try {
    const payload = await state.client.personalitySet(name);
    state.personalityActive = payload.active;
    refreshPersonalityBadge();
    notifySuccess(
      payload.active
        ? `\u{1F3AD} ${payload.active}`
        : `\u{1F3AD} ${t.chrome.personalityDefault}`,
    );
  } catch (error) {
    notifyError(String(error));
  }
}

/** P545: total-token chip for the open session, beside the model
 * badge — sourced from the same /api/usage map as the sidebar rows. */
function refreshTokenBadge(): void {
  const tokens = state.current ? state.sessionTokens.get(state.current.id) ?? 0 : 0;
  if (tokens > 0) {
    el.tokenBadge.textContent = `${formatTokens(tokens)} tok`;
    el.tokenBadge.title = fmt(t.session.tokensTitle, { tokens: tokens.toLocaleString() });
    el.tokenBadge.hidden = false;
  } else {
    el.tokenBadge.hidden = true;
    el.tokenBadge.textContent = "";
  }
}

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
  refreshProjectBadge();
  refreshEndBadge();
  refreshTokenBadge();
  refreshComposerPlaceholder();
}

/** P458: end-reason badge beside the project badge (if ended/archived). */
function refreshEndBadge(): void {
  const reason = state.current?.end_reason;
  if (reason) {
    el.endBadge.textContent = `${END_BADGES[reason] ?? "■"} ${reason}`;
    el.endBadge.title = reason;
    el.endBadge.hidden = false;
  } else {
    el.endBadge.hidden = true;
    el.endBadge.textContent = "";
  }
  // P538: the archive header button flips into an unarchive action.
  const archived = reason === "archived";
  const label = archived ? t.palette.unarchiveSession : t.palette.archiveSession;
  el.chatArchive.title = label;
  el.chatArchive.dataset.i18nTitle = archived
    ? "palette.unarchiveSession"
    : "palette.archiveSession";
  el.chatArchive.textContent = archived ? "\u{267B}" : "\u{1F5C4}";
}

/** P429: owning-project badge beside the model badge (if any). */
function refreshProjectBadge(): void {
  const project = state.current?.project;
  if (project) {
    el.projectBadge.textContent = `\u{1F4C1} ${project}`;
    el.projectBadge.title = project;
    el.projectBadge.hidden = false;
  } else {
    el.projectBadge.hidden = true;
    el.projectBadge.textContent = "";
  }
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

/** P441: badge the Runs tab with the number of waiting approvals. */
async function refreshRunsTabBadge(): Promise<void> {
  const tab = document.getElementById("tab-runs");
  if (!tab || !state.client) return;
  try {
    const runs = await state.client.listRuns();
    const waiting = runs.filter((run) => run.status === "waiting_for_approval").length;
    let badge = tab.querySelector<HTMLSpanElement>(".tab-badge");
    if (waiting === 0) {
      badge?.remove();
      tab.title = "";
      return;
    }
    if (!badge) {
      badge = document.createElement("span");
      badge.className = "tab-badge";
      tab.appendChild(badge);
    }
    badge.textContent = String(waiting);
    tab.title = fmt(t.chrome.tabApprovals, { count: String(waiting) });
  } catch {
    // Transient error — leave the badge as-is.
  }
}

/** P452: previous health-probe outcome (null before the first probe). */
let lastHealthOk: boolean | null = null;

async function pollHealth(): Promise<void> {
  if (!state.client) return;
  // P412: time the health probe so the dot tooltip can show latency.
  const probeStartedAt = performance.now();
  const ok = await state.client.health();
  const probeLatencyMs = Math.round(performance.now() - probeStartedAt);
  el.dot.className = "dot " + (ok ? "up" : "down");
  el.dot.title = ok ? t.session.reachable : t.session.unreachable;
  if (ok) {
    // P452: toast the recovery transition.
    if (lastHealthOk === false) notifySuccess(t.chrome.healthRestored);
    lastHealthOk = true;
    const model = await state.client.models();
    if (model) {
      gatewayModel = model;
      refreshModelBadge();
    }
    // P615: one-shot reasoning-effort chip load once the gateway is up.
    if (!state.reasoningLoaded) {
      state.reasoningLoaded = true;
      void loadReasoningState();
    }
    // P620: one-shot personality chip load.
    if (!state.personalityLoaded) {
      state.personalityLoaded = true;
      void loadPersonalityState();
    }
    // P626: one-shot Priority Processing chip load.
    if (!state.fastLoaded) {
      state.fastLoaded = true;
      void loadFastState();
    }
    // P635: one-shot approvals-mode chip load.
    if (!state.approvalsLoaded) {
      state.approvalsLoaded = true;
      void loadApprovalsState();
    }
    // P365: enrich the dot tooltip with the detailed open probe.
    const detailed = await state.client.healthDetailed();
    if (detailed) {
      el.dot.title = [
        `${detailed.service} v${detailed.version}`,
        `${detailed.provider}/${detailed.model}`,
        detailed.auth_required ? t.chrome.dotAuthOn : t.chrome.dotAuthOff,
        t.chrome.dotRuns.replace("{count}", String(detailed.runs_tracked)),
        t.chrome.dotLatency.replace("{ms}", String(probeLatencyMs)),
      ].join(" · ");
    }
    void updateStatusBar();
    // P441: keep the Runs-tab approval badge fresh on every probe.
    void refreshRunsTabBadge();
  } else {
    // P452: toast the loss transition.
    if (lastHealthOk === true) notifyError(t.chrome.healthLost);
    lastHealthOk = false;
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
    const [info, usage, tasks] = await Promise.all([
      state.client.systemInfo(),
      // P370: process token/tool census for the status bar; keep the
      // payload small — only the summary cards matter here.
      state.client.usage(1).catch(() => null),
      // P382: kanban column census for the status bar.
      state.client.kanbanTasks().catch(() => null),
    ]);
    // P385: each segment carries the view it jumps to on click.
    const segs: [string, string][] = [
      [`v${info.version} · ${info.os}/${info.arch} · ${t.chrome.statusUp.replace("{duration}", formatUptime(info.uptime_secs))}`, "doctor"],
    ];
    if (gatewayModel) segs.push([gatewayModel, "doctor"]);
    segs.push([t.chrome.statusSessions.replace("{count}", String(info.sessions)), "sessions"]);
    segs.push([t.chrome.statusRuns.replace("{count}", String(info.active_runs)), "runs"]);
    segs.push([t.chrome.statusPlugins.replace("{count}", String(info.plugins_loaded)), "plugins"]);
    if (usage) {
      segs.push([
        t.chrome.statusTokens
          .replace("{tokens}", formatTokens(usage.process.total_tokens))
          .replace("{calls}", String(usage.process.tool_calls)),
        "usage",
      ]);
    }
    // P633: the open session's context-window usage (hermes status-bar
    // context parity) — snapshot maintained by the chat-header meter.
    if (state.contextBreakdown && state.contextBreakdown.context_max > 0) {
      segs.push([
        t.chrome.statusCtx.replace("{pct}", String(state.contextBreakdown.context_percent)),
        "chat",
      ]);
    }
    if (tasks) {
      const todo = tasks.filter((task) =>
        ["todo", "ready", "scheduled"].includes(task.status),
      ).length;
      const doing = tasks.filter((task) => task.status === "running").length;
      const blocked = tasks.filter((task) => task.status === "blocked").length;
      segs.push([
        t.chrome.statusKanban
          .replace("{todo}", String(todo))
          .replace("{doing}", String(doing))
          .replace("{blocked}", String(blocked)),
        "kanban",
      ]);
    }
    el.statusbar.innerHTML = segs
      .map(([text, target]) => `<span class="statusbar-seg" data-target="${target}">${text}</span>`)
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
    ["/retitle", t.slash.retitle],
    ["/usage", t.slash.usage],
    ["/kanban", t.slash.kanban],
    ["/new", t.slash.newSession],
    ["/clear", t.slash.clear],
    ["/insights", t.slash.insights],
    ["/compress", t.slash.compress],
    ["/doctor", t.slash.doctor],
    ["/runs", t.slash.runs],
    ["/projects", t.slash.projects],
    ["/config", t.slash.config],
    ["/webhooks", t.slash.webhooks],
    ["/models", t.slash.models],
    ["/plugins", t.slash.plugins],
    ["/archive", t.slash.archive],
    ["/unarchive", t.slash.unarchive],
    ["/export", t.slash.exportSession],
    ["/fork", t.slash.forkSession],
    ["/search", t.slash.search],
    ["/open", t.slash.open],
    ["/status", t.slash.status],
    ["/context", t.slash.context],
    ["/whoami", t.slash.whoami],
    ["/version", t.slash.version],
    ["/commands", t.slash.commands],
    ["/profile", t.slash.profile],
    ["/model", t.slash.model],
    ["/reasoning", t.slash.reasoning],
    ["/memory", t.slash.memory],
    ["/sessions", t.slash.sessions],
    ["/stop", t.slash.stop],
    ["/retry", t.slash.retry],
    ["/undo", t.slash.undo],
    ["/verbose", t.slash.verbose],
    ["/yolo", t.slash.yolo],
    ["/personality", t.slash.personality],
    ["/goal", t.slash.goal],
    ["/subgoal", t.slash.subgoal],
    ["/reload-mcp", t.slash.reloadMcp],
    ["/fast", t.slash.fast],
    ["/branch", t.slash.branch],
    ["/diff", t.slash.diff],
    ["/rollback", t.slash.rollback],
    ["/blueprint", t.slash.blueprint],
    ["/cron", t.slash.cron],
    ["/suggestions", t.slash.suggestions],
    ["/init", t.slash.init],
    ["/agents", t.slash.agents],
    ["/journey", t.slash.journey],
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
  refreshCharCount();
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
let activeSwitchView: ((view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing" | "profiles") => void) | null = null;

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

/**
 * P435/P437: OS-level notification (Web Notifications API). Only used when
 * the window is unfocused — the in-app stacks cover the focused case — and
 * lazily requests permission on first use. Click focuses the window and
 * jumps to the runs view.
 */
function systemNotify(title: string, body: string, tag: string): void {
  if (!state.settings.notifySystem || typeof Notification === "undefined") return;
  const show = () => {
    const note = new Notification(title, {
      body: body || undefined,
      tag: tag || undefined,
    });
    note.onclick = () => {
      window.focus();
      activeSwitchView?.("runs");
      note.close();
    };
  };
  if (Notification.permission === "granted") {
    show();
  } else if (Notification.permission === "default") {
    void Notification.requestPermission().then((permission) => {
      if (permission === "granted") show();
    });
  }
}

function handleDesktopEvent(envelope: DesktopEnvelope): void {
  const payload = envelope.payload ?? {};
  const switchView = (view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing" | "profiles") =>
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
    case "run.completed":
    case "run.failed": {
      const failed = envelope.event === "run.failed";
      const snippet = String(payload.snippet ?? "").trim();
      const runId = String(payload.run_id ?? "");
      notify({
        kind: failed ? "error" : "success",
        title: failed ? t.bridge.runFailed : t.bridge.runCompleted,
        message: snippet || (failed ? t.bridge.runFailed : t.bridge.runCompleted),
        meta: runId ? `#${runId.slice(0, 8)}` : undefined,
        action: { label: t.bridge.runOpenRuns, onClick: () => switchView("runs") },
      });
      if (!document.hasFocus()) {
        systemNotify(
          failed ? t.bridge.runFailed : t.bridge.runCompleted,
          snippet,
          runId ? `ulnclaw-run-${runId}` : "",
        );
      }
      // P467: a settled run changed message counts + the transcript —
      // refresh the sidebar, the sessions-browser list, and (when the
      // settled session is open there) its transcript in place.
      void refreshSessions();
      void state.sessionsBrowser?.refresh();
      state.sessionsBrowser?.refreshTranscript(envelope.session_id);
      // P472: settle the run row + approval badge without waiting for the
      // Runs-view poll.
      void state.runs?.refresh();
      void refreshRunsTabBadge();
      // P474: job rows track last-run state — settle them instantly too.
      void state.jobs?.refresh();
      break;
    }
    case "run.approval": {
      const command = String(payload.command ?? "").trim();
      const runId = String(payload.run_id ?? "");
      notify({
        kind: "warning",
        title: t.bridge.approvalNeeded,
        message: command || t.bridge.approvalNeeded,
        meta: runId ? `#${runId.slice(0, 8)}` : undefined,
        action: { label: t.bridge.approvalReview, onClick: () => switchView("runs") },
      });
      if (!document.hasFocus()) {
        systemNotify(t.bridge.approvalNeeded, command, runId ? `ulnclaw-approval-${runId}` : "");
      }
      // P472: the waiting-approval badge and run rows update immediately.
      void state.runs?.refresh();
      void refreshRunsTabBadge();
      break;
    }
    case "session.message": {
      // P493: a message was appended anywhere — the sessions browser
      // catches up when the affected session is open (debounced there).
      state.sessionsBrowser?.notifyMessageAppended(envelope.session_id);
      // P496: sessions that grow while not open become unread.
      const browserReading =
        state.view === "sessions" &&
        state.sessionsBrowser?.selectedId() === envelope.session_id;
      if (
        envelope.session_id &&
        state.current?.id !== envelope.session_id &&
        !browserReading &&
        !unreadSessions.has(envelope.session_id)
      ) {
        unreadSessions.add(envelope.session_id);
        persistUnread();
        renderSessions();
        renderUnreadBadge();
      }
      // P494: message counts + activity follow along in the sidebar and
      // the sessions-browser list (debounced to coalesce bursts).
      if (sessionMessageListTimer === null) {
        sessionMessageListTimer = window.setTimeout(() => {
          sessionMessageListTimer = null;
          void refreshSessions();
          void state.sessionsBrowser?.refresh();
        }, 2000);
      }
      // P495: the chat view catches up when the open session grows
      // elsewhere and no turn is streaming here.
      if (state.current && envelope.session_id === state.current.id && !state.busy) {
        if (chatCatchupTimer === null) {
          chatCatchupTimer = window.setTimeout(() => {
            chatCatchupTimer = null;
            if (state.current && !state.busy) void openSession(state.current);
          }, 1500);
        }
      }
      break;
    }
    case "session.created":
    case "session.updated":
    case "session.deleted": {
      // P444/P445: another client (CLI/API/cron) created/renamed/archived/
      // deleted a session — refresh the sidebar list and the sessions
      // browser if it is mounted.
      void refreshSessions();
      void state.sessionsBrowser?.refresh();
      // P473: toast cross-client lifecycle events; suppress our own actions
      // (desktop-created sessions carry source "desktop"; deletes are
      // tracked in the local registries).
      const eventSessionId = String(payload.session_id ?? "");
      if (envelope.event === "session.created" && String(payload.source ?? "") !== "desktop") {
        notify({ kind: "success", title: t.bridge.sessionCreated, message: eventSessionId.slice(0, 12) });
      } else if (envelope.event === "session.deleted" && eventSessionId) {
        const local =
          localDeletes.delete(eventSessionId) ||
          (state.sessionsBrowser?.takeLocalDelete(eventSessionId) ?? false);
        if (!local) {
          notify({ kind: "warning", title: t.bridge.sessionDeleted, message: eventSessionId.slice(0, 12) });
        }
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
    // P547: mod+shift+a — archive / unarchive the current session.
    if (mod && event.shiftKey && key === "a") {
      event.preventDefault();
      if (state.current) {
        if (state.current.end_reason === "archived") void runUnarchiveSession();
        else void runArchiveSession();
      }
      return;
    }
    // P547: mod+shift+e — open the export format picker.
    if (mod && event.shiftKey && key === "e") {
      event.preventDefault();
      if (state.current) openExportPicker();
      return;
    }
    // P590: mod+shift+t — toggle the chat file-tree sidebar.
    if (mod && event.shiftKey && key === "t") {
      event.preventDefault();
      void toggleFileTree();
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
      toggleSidebar();
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

/** P537: gate for the wake indicator — armed after the first healthy
 * boot so cold-start failures stay on the boot-failure surface. */
let wakeArmed = false;

async function start(): Promise<void> {
  // Translate the static chrome for any persisted non-en locale (P251).
  applyStatic();
  state.bridge = await loadBridge();
  state.client = new GatewayClient(state.settings);
  void listenMenuEvents();

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
      wakeArmed = true;
      hideConnecting();
      resolveBootFailure();
      void state.onboarding!.maybeOpen();
      return;
    }
    showConnecting();
    for (let attempt = 0; attempt < 40; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      if (await state.client!.health()) {
        wakeArmed = true;
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
  // P537: wake indicator — after sleep/lid-close (30 s+ hidden) or a
  // network return, probe the gateway, silently re-sync the session
  // list, and toast outages until the gateway answers again.
  initWakeIndicator(
    () => state.client?.health() ?? Promise.resolve(false),
    {
      armed: () => wakeArmed,
      onHealthy: (afterOutage) => {
        if (afterOutage) notifySuccess(t.wake.reconnected);
        void refreshSessions();
      },
      onUnreachable: () => notifyError(t.wake.unreachable),
    },
  );
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
    if (state.current) {
      state.drafts.set(state.current.id, el.input.value);
      persistDrafts();
    }
    state.current = null;
    state.contextBreakdown = null;
    el.contextMeter.hidden = true;
    el.contextPop.hidden = true;
    el.dayJump.hidden = true;
    el.chatActions.hidden = true;
    el.chatTitle.textContent = t.session.newTitle;
    refreshWindowTitle();
    el.messages.innerHTML = "";
    renderIntro(el.messages, "new");
    renderSessions();
    refreshComposerPlaceholder();
    el.input.value = "";
    refreshCharCount();
    el.input.focus();
  };

  // P372: live sidebar session filter.
  el.sessionFilter.addEventListener("input", () => {
    state.sessionFilterText = el.sessionFilter.value;
    try {
      localStorage.setItem(SESSION_FILTER_KEY, el.sessionFilter.value);
    } catch {
      /* storage unavailable — filter stays session-scoped */
    }
    renderSessions();
  });

  // P407: session sort toggle (activity ↔ title), persisted.
  el.sessionSort.onclick = () => {
    state.sessionSortMode = state.sessionSortMode === "activity" ? "title" : "activity";
    try {
      localStorage.setItem(SESSION_SORT_KEY, state.sessionSortMode);
    } catch {
      /* storage unavailable */
    }
    refreshSortButton();
    renderSessions();
  };
  refreshSortButton();

  // P414: hide/show archived sessions in the sidebar, persisted
  // (P424: the same toggle rides the command palette).
  el.sessionArchivedToggle.onclick = () => toggleHideArchived();
  refreshArchivedButton();

  // P394: keyboard navigation over the sidebar session list.
  el.sessionList.addEventListener("keydown", (event) => {
    const count = state.sessionListVisible.length;
    if (count === 0) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      state.sessionCursor =
        event.key === "ArrowDown"
          ? Math.min(count - 1, state.sessionCursor + 1)
          : Math.max(0, state.sessionCursor - 1);
      renderSessions();
      el.sessionList
        .querySelector(".session-item.cursor")
        ?.scrollIntoView({ block: "nearest" });
    } else if (event.key === "Enter") {
      const session = state.sessionListVisible[state.sessionCursor];
      if (session) {
        event.preventDefault();
        void openSession(session);
      }
    }
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
    // P415: pick the export format (md / html / portable json).
    openExportPicker();
  };
  el.chatDelete.onclick = () => {
    if (state.current) void deleteSession(state.current);
  };
  // P538: fork + archive/unarchive join the chat-header action set.
  el.chatFork.onclick = () => {
    void runForkSession();
  };
  el.chatArchive.onclick = () => {
    if (!state.current) return;
    if (state.current.end_reason === "archived") void runUnarchiveSession();
    else void runArchiveSession();
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
    refreshCharCount();
    return true;
  }
  if (event.key === "ArrowDown") {
    if (state.composerHistoryIndex === null) return false;
    event.preventDefault();
    if (state.composerHistoryIndex < history.length - 1) {
      state.composerHistoryIndex += 1;
      el.input.value = history[state.composerHistoryIndex];
      refreshCharCount();
    } else {
      state.composerHistoryIndex = null;
      el.input.value = state.composerDraft;
      refreshCharCount();
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
  el.input.addEventListener("input", () => {
    renderSlashPop();
    refreshCharCount();
  });
  el.input.addEventListener("paste", (event) => void handlePasteImages(event));
  el.settingsBtn.onclick = () => {
    el.settingUrl.value = state.settings.url;
    el.settingKey.value = state.settings.key;
    el.settingManage.checked = state.settings.manage;
    // P431: the reopen-last-session launch toggle.
    el.settingReopen.checked = state.settings.reopenLast;
    // P434: composer warn threshold.
    el.settingCharWarn.value = String(state.settings.charWarn);
    // P466: composer hard limit (0 = off).
    el.settingCharLimit.value = String(state.settings.charLimit);
    // P435: OS-level settle notifications toggle.
    el.settingNotifySystem.checked = state.settings.notifySystem;
    el.settings.showModal();
    // P634: personalities surface — fill the picker from the gateway and
    // hide the row when no personas are configured.
    void fillPersonalityPicker();
  };
  el.notifyBell.onclick = () => openNotifyHistory();
  // P360 + P543: a click opens the session-info popover; a double-click
  // starts an inline rename instead. The debounce keeps the popover
  // closed while the second click of the pair lands.
  let titleClickTimer: number | null = null;
  el.chatTitle.addEventListener("click", () => {
    if (titleClickTimer !== null) return;
    titleClickTimer = window.setTimeout(() => {
      titleClickTimer = null;
      void openSessionInfo();
    }, 260);
  });
  el.chatTitle.addEventListener("dblclick", () => {
    if (titleClickTimer !== null) {
      window.clearTimeout(titleClickTimer);
      titleClickTimer = null;
    }
    beginInlineRename();
  });
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
      reopenLast: el.settingReopen.checked,
      charWarn: Number(el.settingCharWarn.value) > 0
        ? Math.floor(Number(el.settingCharWarn.value))
        : 4000,
      charLimit: Number(el.settingCharLimit.value) > 0
        ? Math.floor(Number(el.settingCharLimit.value))
        : 0,
      notifySystem: el.settingNotifySystem.checked,
    };
    saveSettings(next);
    state.settings = next;
    state.client = new GatewayClient(next);
    startDesktopEvents();
    // P634: persist the personality pick alongside the other settings.
    void applySettingsPersonality();
    void pollHealth();
    void refreshSessions();
    renderUnreadBadge();
    void loadAppearance();
    refreshCharCount();
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
  const profilesMain = document.getElementById("profiles")!;
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
  const tabProfiles = document.getElementById("tab-profiles") as HTMLButtonElement;
  state.kanban = new KanbanWidget(kanbanMain, () => state.client);
  state.kanban.mount();
  state.projects = new ProjectsWidget(
    projectsMain,
    () => state.client,
    // P427: create a cwd-bound session for the project and open it.
    (cwd, title) => {
      if (!state.client) return;
      void state.client
        .createSession({ cwd, title })
        .then((session) => {
          state.sessions.unshift(session);
          void openSession(session);
          switchView("chat");
        })
        .catch((error) => {
          notifyError(fmt(t.session.createFailed, { error: String(error) }));
        });
    },
  );
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
  state.runs = new RunsWidget(
    runsMain,
    () => state.client,
    // P425: run cards jump into their session in the chat view.
    (sessionId) => {
      const known = state.sessions.find((session) => session.id === sessionId);
      const done = known
        ? Promise.resolve(known)
        : state.client!.getSession(sessionId).catch(() => null);
      void done.then((session) => {
        if (!session) return;
        if (!state.sessions.some((candidate) => candidate.id === session.id)) {
          state.sessions.unshift(session);
        }
        void openSession(session);
        switchView("chat");
      });
    },
  );
  state.runs.mount();
  state.skillsView = new SkillsWidget(skillsMain, () => state.client);
  state.skillsView.mount();
  state.sessionsBrowser = new SessionsViewWidget(
    sessionsViewMain,
    () => state.client,
    // P422: resume a browsed session straight from the Sessions view.
    (session) => {
      void openSession(session);
      switchView("chat");
    },
    // P496: unread tracking hooks.
    (sessionId) => unreadSessions.has(sessionId),
    (sessionId) => {
      if (unreadSessions.delete(sessionId)) {
        persistUnread();
        renderSessions();
        renderUnreadBadge();
      }
    },
    // P556: pin hooks over the P548 persisted pin set.
    (sessionId) => pinnedSessions.has(sessionId),
    (session) => togglePinSession(session),
  );
  state.sessionsBrowser.mount();
  state.modelsView = new ModelsViewWidget(modelsMain, () => state.client);
  state.modelsView.mount();
  state.pluginsView = new PluginsViewWidget(pluginsMain, () => state.client);
  state.pluginsView.mount();
  state.pairingView = new PairingViewWidget(pairingMain, () => state.client);
  state.pairingView.mount();
  state.profilesView = new ProfilesViewWidget(profilesMain, () => state.client);
  state.profilesView.mount();
  const switchView = (view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing" | "profiles") => {
    if (view !== "chat") state.findBar?.close();
    state.view = view;
    try {
      localStorage.setItem(ACTIVE_VIEW_KEY, view);
    } catch {
      /* storage unavailable — view stays session-scoped */
    }
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
    profilesMain.hidden = view !== "profiles";
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
    tabProfiles.classList.toggle("active", view === "profiles");
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
    if (view === "profiles") {
      state.profilesView!.start();
    } else {
      state.profilesView!.stop();
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
  tabProfiles.onclick = () => switchView("profiles");

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
  el.reasoningBadge.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleReasoningPop();
  });
  el.reasoningPop.addEventListener("click", (event) => {
    const item = (event.target as HTMLElement).closest(".slash-item") as HTMLElement | null;
    if (!item) return;
    const level = item.dataset.level ?? "";
    void applyReasoningPick(level === "" ? null : level);
  });
  document.addEventListener("click", (event) => {
    if (el.reasoningPop.hidden) return;
    const target = event.target as Node;
    if (el.reasoningPop.contains(target) || el.reasoningBadge.contains(target)) return;
    el.reasoningPop.hidden = true;
  });
  el.personalityBadge.addEventListener("click", (event) => {
    event.stopPropagation();
    togglePersonalityPop();
  });
  el.contextMeter.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleContextPop();
  });
  el.fastBadge.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleFastPop();
  });
  el.fastPop.addEventListener("click", (event) => {
    const item = (event.target as HTMLElement).closest(".slash-item") as HTMLElement | null;
    if (!item) return;
    const mode = item.dataset.mode;
    if (mode === "fast" || mode === "normal") void applyFastPick(mode);
  });
  document.addEventListener("click", (event) => {
    if (el.fastPop.hidden) return;
    const target = event.target as Node;
    if (el.fastPop.contains(target) || el.fastBadge.contains(target)) return;
    el.fastPop.hidden = true;
  });
  el.approvalsBadge.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleApprovalsPop();
  });
  el.approvalsPop.addEventListener("click", (event) => {
    const item = (event.target as HTMLElement).closest(".slash-item") as HTMLElement | null;
    if (!item) return;
    const mode = item.dataset.mode;
    if (mode) void applyApprovalsPick(mode);
  });
  document.addEventListener("click", (event) => {
    if (el.approvalsPop.hidden) return;
    const target = event.target as Node;
    if (el.approvalsPop.contains(target) || el.approvalsBadge.contains(target)) return;
    el.approvalsPop.hidden = true;
  });
  document.addEventListener("click", (event) => {
    if (el.contextPop.hidden) return;
    const target = event.target as Node;
    if (el.contextPop.contains(target) || el.contextMeter.contains(target)) return;
    el.contextPop.hidden = true;
  });
  el.personalityPop.addEventListener("click", (event) => {
    const item = (event.target as HTMLElement).closest(".slash-item") as HTMLElement | null;
    if (!item) return;
    const name = item.dataset.name ?? "";
    void applyPersonalityPick(name === "" ? null : name);
  });
  document.addEventListener("click", (event) => {
    if (el.personalityPop.hidden) return;
    const target = event.target as Node;
    if (el.personalityPop.contains(target) || el.personalityBadge.contains(target)) return;
    el.personalityPop.hidden = true;
  });
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
  state.fileTree = new FileTreePanel(() => state.client, (path, name) => void attachFsTreeFile(path, name));
  state.fileTree.mount(document.getElementById("chat-body")!);
  (document.getElementById("chat-file-tree") as HTMLButtonElement).addEventListener("click", () => {
    void toggleFileTree();
  });

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
    isPinned: (id) => pinnedSessions.has(id),
    togglePin: (session) => togglePinSession(session),
    isUnread: (id) => unreadSessions.has(id),
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
    retitleSession: () => runRetitleSession(),
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
    kanbanBoardSwitcher: () => openKanbanBoardPicker(() => switchView("kanban")),
    toggleSidebar: () => toggleSidebar(),
    themePicker: () => openThemePicker(),
    fontPicker: () => openFontPicker(),
    copySessionId: () => runCopySessionId(),
    copyLastReply: () => runCopyLastReply(),
    copyTranscript: () => runCopyTranscript(),
    forkSession: () => runForkSession(),
    retitleSkills: () => runRetitleSkills(),
    toggleUnreadOnly: () => toggleUnreadOnly(),
    // P548: pin the open session to the top of the sidebar list.
    togglePinSession: () => {
      if (state.current) togglePinSession(state.current);
    },
    toggleHideArchived: () => toggleHideArchived(),
    // P498: clear every unread marker at once.
    markAllRead: () => {
      if (unreadSessions.size === 0) return;
      unreadSessions.clear();
      persistUnread();
      renderSessions();
      renderUnreadBadge();
      void state.sessionsBrowser?.refresh();
    },
    newSessionInProject: () => runNewSessionInProject(),
    // P448: jump into the sessions browser with the chat session selected.
    openInSessionsBrowser: () => {
      const id = state.current?.id;
      if (!id) return;
      switchView("sessions");
      state.sessionsBrowser?.openSession(id);
    },
    archiveSession: () => runArchiveSession(),
    unarchiveSession: () => runUnarchiveSession(),
    toggleFileTree: () => toggleFileTree(),
    quickEntry: () => state.quickEntry?.toggle(),
  });

  // Quick Entry (hermes quick-entry parity): Ctrl/Cmd+Shift+Q opens a
  // one-input capture composer targeting the current, a new, or a recent
  // session; the text rides the primary composer's submit path.
  state.quickEntry = new QuickEntry({
    connected: () => state.client !== null,
    sessions: () => state.sessions,
    currentSessionId: () => state.current?.id ?? null,
    openSession: async (session) => {
      await openSession(session);
    },
    newSession: () => el.newSession.click(),
    sendText: (text) => {
      switchView("chat");
      el.input.value = text;
      refreshCharCount();
      void sendTurn();
    },
  });
  state.quickEntry.mount();

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
  // P385: status-bar segments jump to their view (doctor fallback).
  // P388: restore the persisted sidebar collapse state first.
  if (localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1") {
    document.getElementById("app")!.classList.add("no-sidebar");
  }
  // P395: restore persisted composer drafts.
  loadPersistedDrafts();
  // P403: restore persisted notification history.
  loadPersistedNotificationHistory();
  // P396: restore persisted prompt-recall history.
  loadPersistedComposerHistory();
  // P397: restore the persisted sidebar session filter.
  const savedFilter = localStorage.getItem(SESSION_FILTER_KEY);
  if (savedFilter) {
    state.sessionFilterText = savedFilter;
    el.sessionFilter.value = savedFilter;
  }
  // P402: Alt+↑/Alt+↓ jumps between day dividers in the chat view.
  window.addEventListener("keydown", (event) => {
    if (
      state.view === "chat" &&
      event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.shiftKey &&
      (event.key === "ArrowUp" || event.key === "ArrowDown")
    ) {
      event.preventDefault();
      jumpDayDivider(event.key === "ArrowDown" ? 1 : -1);
    }
  });
  // P406: PageUp/PageDown pages the chat transcript when the focus is
  // not inside a text field.
  window.addEventListener("keydown", (event) => {
    if (state.view !== "chat") return;
    if (event.key !== "PageUp" && event.key !== "PageDown") return;
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "TEXTAREA" || target.tagName === "INPUT")) return;
    event.preventDefault();
    const step = el.messages.clientHeight * 0.9;
    el.messages.scrollTop += event.key === "PageDown" ? step : -step;
  });
  // P408: End/Home jump straight to the newest / oldest chat message
  // when the focus is not inside a text field.
  window.addEventListener("keydown", (event) => {
    if (state.view !== "chat") return;
    if (event.key !== "End" && event.key !== "Home") return;
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "TEXTAREA" || target.tagName === "INPUT")) return;
    event.preventDefault();
    if (event.key === "End") {
      el.messages.scrollTop = el.messages.scrollHeight;
      el.scrollBottom.hidden = true;
    } else {
      el.messages.scrollTop = 0;
    }
  });
  // P552: F5 refreshes the active view in place (chat reloads the open
  // transcript; widget views call their own refresh entry points).
  const refreshActiveView = (): void => {
    switch (state.view) {
      case "chat":
        renderSessions();
        if (state.current && !state.busy) void openSession(state.current);
        break;
      case "kanban":
        void state.kanban?.refresh();
        break;
      case "projects":
        (document.querySelector("#projects-refresh") as HTMLButtonElement | null)?.click();
        break;
      case "jobs":
        void state.jobs?.refresh();
        break;
      case "usage":
        void state.usage?.refresh();
        break;
      case "config":
        void state.config?.refresh();
        break;
      case "doctor":
        (document.querySelector("#doctor-run") as HTMLButtonElement | null)?.click();
        break;
      case "webhooks":
        void state.webhooks?.refresh();
        break;
      case "runs":
        void state.runs?.refresh();
        break;
      case "skills":
        void state.skillsView?.refresh();
        break;
      case "sessions":
        void state.sessionsBrowser?.refresh();
        break;
      case "models":
        void state.modelsView?.refresh();
        break;
      case "plugins":
        void state.pluginsView?.refresh();
        break;
      case "pairing":
        void state.pairingView?.refresh();
        break;
      case "profiles":
        void state.profilesView?.refresh();
        break;
    }
  };
  window.addEventListener("keydown", (event) => {
    if (event.key !== "F5") return;
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "TEXTAREA" || target.tagName === "INPUT")) return;
    event.preventDefault();
    refreshActiveView();
  });
  // P398: reopen the last active view.
  const savedView = localStorage.getItem(ACTIVE_VIEW_KEY);
  if (
    savedView === "chat" || savedView === "kanban" || savedView === "projects" ||
    savedView === "jobs" || savedView === "usage" || savedView === "config" ||
    savedView === "doctor" || savedView === "webhooks" || savedView === "runs" ||
    savedView === "skills" || savedView === "sessions" || savedView === "models" ||
    savedView === "plugins" || savedView === "pairing" ||
    savedView === "profiles"
  ) {
    switchView(savedView);
  }
  el.statusbar.addEventListener("click", (event) => {
    const seg = (event.target as HTMLElement).closest<HTMLElement>(".statusbar-seg");
    const target = seg?.dataset.target;
    switchView(
      target === "sessions" ||
        target === "runs" ||
        target === "plugins" ||
        target === "usage" ||
        target === "kanban"
        ? target
        : "doctor",
    );
  });
  await refreshSessions();
  // P399: reopen the last open session (when it still exists) —
  // P431: gated by the settings toggle.
  const lastSessionId = localStorage.getItem(LAST_SESSION_KEY);
  if (state.settings.reopenLast && lastSessionId) {
    const last = state.sessions.find((row) => row.id === lastSessionId);
    if (last) await openSession(last);
  }
  state.skills = (await state.client.listSkills()) || [];
  setInterval(() => void pollHealth(), 10000);
  setInterval(() => void refreshSessions(), 30000);
  startApprovalWatcher();
}

/** P563: compact duration (45s / 12m / 3h05m / 2d4h) — TUI P562 parity. */
function formatDurationCompact(totalSecs: number): string {
  if (totalSecs < 60) return `${totalSecs}s`;
  const minutes = Math.floor(totalSecs / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h${String(minutes % 60).padStart(2, "0")}m`;
  return `${Math.floor(hours / 24)}d${hours % 24}h`;
}

/** P564: HTML escape for the popover's message-snippet values. */
function escapeHtmlInfo(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** P360: session info popover — clicking the chat title shows the
 * current session's metadata (id, source, model, project, activity,
 * message census) with a copy-id action. */
function formatWhen(ms: number | null | undefined): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

async function openSessionInfo(): Promise<void> {
  const session = state.current;
  if (!session) return;
  // P554: prefer the enriched single-session payload (fork lineage +
  // total tokens); fall back to the cached row if the fetch fails.
  let detail: SessionRow = session;
  if (state.client) {
    try {
      const enriched = await state.client.getSession(session.id);
      if (enriched) detail = enriched;
    } catch {
      /* gateway unreachable — show what we have */
    }
  }
  const rows: [string, string][] = [
    [t.session.infoId, detail.id],
    [t.session.infoSource, detail.source],
    [t.session.infoModel, detail.model || gatewayModel || "—"],
    [t.session.infoProject, detail.project || "—"],
    [t.session.infoStarted, formatWhen(detail.started_at)],
    [t.session.infoActivity, formatWhen(detail.last_activity_at)],
    [t.session.infoMessages, String(detail.message_count ?? "—")],
  ];
  // P563: session duration (started → last activity), TUI parity (P562).
  const durationSecs = Math.max(0, Math.floor(detail.last_activity_at - detail.started_at));
  if (durationSecs > 0) {
    rows.push([t.session.infoDuration, formatDurationCompact(durationSecs)]);
  }
  // P460: show the end reason when the session is ended/archived.
  if (detail.end_reason) {
    rows.push([
      t.session.infoEndReason,
      `${END_BADGES[detail.end_reason] ?? "■"} ${detail.end_reason}`,
    ]);
  }
  if (typeof detail.total_tokens === "number") {
    rows.push([t.session.infoTokens, formatTokens(detail.total_tokens)]);
  }
  // P627: live context-in-use (breakdown endpoint's summary fields).
  if (detail.context && detail.context.max > 0) {
    rows.push([
      t.session.infoContext,
      `${formatTokens(detail.context.used)} / ${formatTokens(detail.context.max)} (${detail.context.percent}%)`,
    ]);
  }
  // P561: per-role message census from the enriched payload.
  if (detail.message_counts && Object.keys(detail.message_counts).length) {
    const census = Object.entries(detail.message_counts)
      .map(([role, count]) => `${role}: ${count}`)
      .join(" \u00b7 ");
    rows.push([t.session.infoCensus, census]);
  }
  if (detail.child_session_ids?.length) {
    rows.push([t.session.infoChildren, detail.child_session_ids.join(", ")]);
  }
  // P564: first/last message snippets for at-a-glance context.
  const firstUser = (detail.first_user_message || "").trim();
  if (firstUser) {
    rows.push([t.session.infoFirstUser, `\u201C${escapeHtmlInfo(firstUser)}\u201D`]);
  }
  const lastMessage = (detail.last_message || "").trim();
  if (lastMessage && lastMessage !== firstUser) {
    rows.push([t.session.infoLastMessage, `\u201C${escapeHtmlInfo(lastMessage)}\u201D`]);
  }
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

/** P359/P624: chat-header context meter — live context-window usage
 * from the gateway's breakdown endpoint (hermes context_percent; the
 * old meter showed cumulative throughput tokens, which grow forever and
 * are not context size). */
async function updateContextMeter(): Promise<void> {
  if (!state.client || !state.current) {
    el.contextMeter.hidden = true;
    state.contextBreakdown = null;
    return;
  }
  try {
    const breakdown = await state.client.sessionContextBreakdown(state.current.id);
    state.contextBreakdown = breakdown;
    if (breakdown.context_max <= 0) {
      el.contextMeter.hidden = true;
      return;
    }
    const pct = breakdown.context_percent;
    el.contextMeterFill.style.width = `${pct}%`;
    el.contextMeterText.textContent = `${pct}%`;
    el.contextMeter.title = `${formatTokens(breakdown.context_used)} / ${formatTokens(breakdown.context_max)} tokens`;
    el.contextMeter.classList.toggle("hot", pct >= 80);
    el.contextMeter.hidden = false;
  } catch {
    el.contextMeter.hidden = true;
  }
}

/** P624: category colors for the context popup's stacked bar (hermes
 * --context-usage-* variables). */
const CONTEXT_CATEGORY_COLORS: Record<string, string> = {
  system_prompt: "#7c6cd8",
  tool_definitions: "#4a9eff",
  mcp: "#38b2a3",
  memory: "#d88a3a",
  conversation: "#58b85c",
};

function renderContextPop(): void {
  const breakdown = state.contextBreakdown;
  if (!breakdown || breakdown.context_max <= 0) {
    el.contextPop.innerHTML = `<div class="slash-item">${escapeHtmlInfo(t.contextUsage.noData)}</div>`;
    return;
  }
  const total = breakdown.context_max;
  const segments = breakdown.categories
    .map((category) => {
      const width = Math.max(0.5, (category.tokens / total) * 100);
      const color = CONTEXT_CATEGORY_COLORS[category.id] ?? "var(--accent, #4a9eff)";
      return `<span style="width:${width.toFixed(2)}%;background:${color}" title="${escapeHtmlInfo(category.label)}"></span>`;
    })
    .join("");
  const rows = breakdown.categories
    .map((category) => {
      const pct = ((category.tokens / total) * 100).toFixed(1);
      const color = CONTEXT_CATEGORY_COLORS[category.id] ?? "var(--accent, #4a9eff)";
      return `<div class="notify-history-row"><span><span class="ctx-dot" style="background:${color}"></span>${escapeHtmlInfo(category.label)}</span><span>${formatTokens(category.tokens)} \u00B7 ${pct}%</span></div>`;
    })
    .join("");
  const free = Math.max(0, total - breakdown.estimated_total);
  const freePct = ((free / total) * 100).toFixed(1);
  el.contextPop.innerHTML = [
    `<div class="ctx-pop-title">${escapeHtmlInfo(t.contextUsage.title)}</div>`,
    `<div class="ctx-stack">${segments}</div>`,
    rows,
    `<div class="notify-history-row"><span><span class="ctx-dot"></span>${escapeHtmlInfo(t.contextUsage.freeSpace)}</span><span>${formatTokens(free)} \u00B7 ${freePct}%</span></div>`,
    `<div class="ctx-window">${escapeHtmlInfo(t.contextUsage.window)}: ${formatTokens(breakdown.context_used)} / ${formatTokens(total)} (${breakdown.context_percent}%)</div>`,
  ].join("");
}

async function refreshContextPop(): Promise<void> {
  if (!state.client || !state.current) return;
  try {
    state.contextBreakdown = await state.client.sessionContextBreakdown(state.current.id);
    if (!el.contextPop.hidden) renderContextPop();
  } catch {
    // Keep the last snapshot visible on transient errors.
  }
}

function toggleContextPop(): void {
  if (!el.contextPop.hidden) {
    el.contextPop.hidden = true;
    return;
  }
  renderContextPop();
  const rect = el.contextMeter.getBoundingClientRect();
  el.contextPop.style.position = "fixed";
  el.contextPop.style.left = `${Math.max(8, rect.right - 300)}px`;
  el.contextPop.style.top = `${rect.bottom + 6}px`;
  el.contextPop.style.right = "auto";
  el.contextPop.style.bottom = "auto";
  el.contextPop.style.minWidth = "280px";
  el.contextPop.hidden = false;
  void refreshContextPop();
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
  refreshWindowTitle();
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
    [`${mod}+Shift+A`, t.chrome.scArchive],
    [`${mod}+Shift+E`, t.chrome.scExport],
    [`${mod}+Shift+T`, t.chrome.scFileTree],
    [`${mod}+Shift+Q`, t.chrome.scQuickEntry],
    [`${mod}+,`, t.chrome.scSettings],
    [`${mod}+B`, t.chrome.scSidebar],
    [`${mod}+K`, t.chrome.scPalette],
    [`${mod}+F`, t.chrome.scFind],
    ["Enter", t.chrome.scFocus],
    ["F1", t.chrome.scShortcuts],
    ["F5", t.chrome.scRefreshView],
    ["↑ / ↓", t.chrome.scRecall],
    ["↑ / ↓ + Enter", t.chrome.scSessionNav],
    ["Alt+↑ / Alt+↓", t.chrome.scDayJump],
    ["End / Home", t.chrome.scEndHome],
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

/** P400: palette theme picker — apply + persist dashboard themes. */
async function openThemePicker(): Promise<void> {
  if (!state.client) return;
  let payload: { themes: DashboardTheme[]; active: string };
  try {
    payload = await state.client.dashboardThemes();
  } catch (error) {
    notifyError(fmt(t.palette.themeFailed, { error: String(error) }));
    return;
  }
  const dialog = document.createElement("dialog");
  dialog.className = "theme-picker-dialog";
  const heading = document.createElement("div");
  heading.className = "theme-picker-title";
  heading.textContent = t.palette.themePickerTitle;
  const list = document.createElement("div");
  list.className = "theme-picker-list";
  for (const theme of payload.themes) {
    const row = document.createElement("button");
    row.className = "theme-picker-item" + (theme.name === payload.active ? " active" : "");
    row.textContent = theme.label;
    row.title = theme.description;
    row.onclick = () => {
      applyTheme(theme.name);
      state.client?.dashboardSetTheme(theme.name).catch(() => undefined);
      if (el.settingTheme.value !== theme.name) el.settingTheme.value = theme.name;
      dialog.close();
    };
    list.appendChild(row);
  }
  dialog.append(heading, list);
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.appendChild(dialog);
  dialog.showModal();
}

/** P401: palette font picker — apply + persist the dashboard font. */
async function openFontPicker(): Promise<void> {
  if (!state.client) return;
  let active: string;
  try {
    active = await state.client.dashboardFont();
  } catch (error) {
    notifyError(fmt(t.palette.fontFailed, { error: String(error) }));
    return;
  }
  const dialog = document.createElement("dialog");
  dialog.className = "theme-picker-dialog";
  const heading = document.createElement("div");
  heading.className = "theme-picker-title";
  heading.textContent = t.palette.fontPickerTitle;
  const list = document.createElement("div");
  list.className = "theme-picker-list";
  for (const font of FONT_IDS) {
    const row = document.createElement("button");
    row.className = "theme-picker-item" + (font === active ? " active" : "");
    row.textContent = font.replace(/-/g, " ");
    row.onclick = () => {
      applyFont(font);
      state.client?.dashboardSetFont(font).catch(() => undefined);
      if (el.settingFont.value !== font) el.settingFont.value = font;
      dialog.close();
    };
    list.appendChild(row);
  }
  dialog.append(heading, list);
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.appendChild(dialog);
  dialog.showModal();
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

/** P411: palette board switcher — pick the active kanban board from
 * anywhere (mirrors the board select in the kanban view header). */
async function openKanbanBoardPicker(navigateToKanban: () => void): Promise<void> {
  if (!state.client) return;
  let boards: KanbanBoard[];
  try {
    boards = await state.client.kanbanBoards();
  } catch (error) {
    notifyError(fmt(t.palette.kanbanBoardsFailed, { error: String(error) }));
    return;
  }
  if (!boards.length) {
    notifyError(t.palette.kanbanBoardsEmpty);
    return;
  }
  const dialog = document.createElement("dialog");
  dialog.className = "theme-picker-dialog";
  const heading = document.createElement("div");
  heading.className = "theme-picker-title";
  heading.textContent = t.palette.kanbanBoardSwitch;
  const list = document.createElement("div");
  list.className = "theme-picker-list";
  for (const board of boards) {
    const row = document.createElement("button");
    row.className = "theme-picker-item" + (board.current ? " active" : "");
    row.textContent = `${board.name} (${board.open_tasks}/${board.total_tasks})`;
    row.title = board.slug;
    row.onclick = () => {
      dialog.close();
      const done = board.current
        ? Promise.resolve(true)
        : state.client!
            .kanbanSwitchBoard(board.slug)
            .then((ok) => {
              if (ok) notifySuccess(fmt(t.palette.kanbanBoardSwitched, { board: board.name }));
              return ok;
            })
            .catch((error) => {
              notifyError(fmt(t.palette.kanbanBoardsFailed, { error: String(error) }));
              return false;
            });
      void done.then((ok) => {
        if (!ok) return;
        void state.kanban?.refresh();
        navigateToKanban();
      });
    };
    list.appendChild(row);
  }
  dialog.append(heading, list);
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.appendChild(dialog);
  dialog.showModal();
}

/** P405: palette action — copy the current session's ID. */
async function runCopySessionId(): Promise<void> {
  if (!state.current) return;
  try {
    await navigator.clipboard.writeText(state.current.id);
    notifySuccess(t.session.infoCopied);
  } catch {
    notifyError(t.session.infoCopyFailed);
  }
}

/** P418: palette action — copy the last assistant reply in the
 * open transcript (the same text the per-bubble ⧉ action copies). */
async function runCopyLastReply(): Promise<void> {
  const bubbles = el.messages.querySelectorAll(".message.assistant .bubble");
  const last = bubbles[bubbles.length - 1];
  const text = last?.textContent?.trim();
  if (!text) {
    notifyError(t.palette.lastReplyNone);
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
    notifySuccess(t.palette.lastReplyCopied);
  } catch {
    notifyError(t.session.copyFailed);
  }
}

/** P525: palette action — copy the current session transcript as
 * Markdown straight to the clipboard (no file download). */
async function runCopyTranscript(): Promise<void> {
  if (!state.client || !state.current) return;
  try {
    const { blob } = await state.client.exportSession(state.current.id, "md");
    const text = await blob.text();
    await navigator.clipboard.writeText(text);
    notifySuccess(t.palette.copiedTranscript);
  } catch (error) {
    notifyError(fmt(t.palette.copyTranscriptFailed, { error: String(error) }));
  }
}

/** P420: palette action — fork the current session
 * (POST /api/sessions/:id/fork) and jump into the branch. */
async function runForkSession(): Promise<void> {
  if (!state.client || !state.current) return;
  const label = state.current.title || state.current.id.slice(0, 8);
  try {
    const forked = await state.client.forkSession(state.current.id);
    notifySuccess(fmt(t.palette.sessionForked, { label }));
    await refreshSessions();
    await openSession(forked);
  } catch (error) {
    notifyError(fmt(t.palette.forkFailed, { error: String(error) }));
  }
}

/** P531: palette action — preview leaked `/skill` scaffold titles
 * (dry-run over POST /api/sessions/retitle-skills) and apply the
 * regenerated ones from the review dialog. */
async function runRetitleSkills(): Promise<void> {
  if (!state.client) return;
  let preview: RetitleSkillsReport;
  try {
    preview = await state.client.retitleSkills(50, false);
  } catch (error) {
    notifyError(fmt(t.palette.retitleFailed, { error: String(error) }));
    return;
  }
  if (preview.sessions.length === 0) {
    notifySuccess(t.palette.retitleNone);
    return;
  }
  const dialog = document.createElement("dialog");
  dialog.className = "theme-picker-dialog";
  const heading = document.createElement("div");
  heading.className = "theme-picker-title";
  heading.textContent = fmt(t.palette.retitleTitle, { count: preview.scanned });
  const list = document.createElement("div");
  list.className = "theme-picker-list";
  for (const row of preview.sessions) {
    const item = document.createElement("div");
    item.className = "theme-picker-item";
    const old = row.old_title || row.id.slice(0, 8);
    if (row.status === "rejected") {
      item.style.opacity = "0.6";
      item.textContent = `${old} \u{2192} ${row.new_title} (${t.palette.retitleRejected})`;
    } else {
      item.textContent = `${old} \u{2192} ${row.new_title}`;
    }
    list.appendChild(item);
  }
  const footer = document.createElement("div");
  footer.style.display = "flex";
  footer.style.gap = "8px";
  footer.style.marginTop = "10px";
  const applyBtn = document.createElement("button");
  applyBtn.className = "primary";
  applyBtn.textContent = t.palette.retitleApply;
  applyBtn.disabled = preview.sessions.every((row) => row.status === "rejected");
  applyBtn.onclick = () => {
    applyBtn.disabled = true;
    state.client
      ?.retitleSkills(50, true)
      .then((result) => {
        notifySuccess(fmt(t.palette.retitleApplied, { count: result.applied }));
        dialog.close();
        void refreshSessions();
      })
      .catch((error) => {
        applyBtn.disabled = false;
        notifyError(fmt(t.palette.retitleFailed, { error: String(error) }));
      });
  };
  const closeBtn = document.createElement("button");
  closeBtn.className = "ghost";
  closeBtn.textContent = t.palette.retitleClose;
  closeBtn.onclick = () => dialog.close();
  footer.append(applyBtn, closeBtn);
  dialog.append(heading, list, footer);
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.appendChild(dialog);
  dialog.showModal();
}

/** P428: palette action — start a session rooted at the active
 * project's folder (primary path first, else the first folder). */
async function runNewSessionInProject(): Promise<void> {
  if (!state.client) return;
  let active: Project | undefined;
  try {
    const listing = await state.client.projectsList();
    active = listing.projects.find((project) => project.id === listing.active_id);
  } catch (error) {
    notifyError(fmt(t.palette.projectsLoadFailed, { error: String(error) }));
    return;
  }
  const cwd = active?.primary_path ?? active?.folders[0]?.path;
  if (!active || !cwd) {
    notifyError(t.palette.noActiveProject);
    return;
  }
  try {
    const session = await state.client.createSession({ cwd, title: active.name });
    state.sessions.unshift(session);
    notifySuccess(fmt(t.palette.projectSessionCreated, { label: active.name }));
    await openSession(session);
  } catch (error) {
    notifyError(fmt(t.session.createFailed, { error: String(error) }));
  }
}

/** P541: sidebar hover action — archive/unarchive any row's session,
 * mirroring the palette flows but scoped to the clicked row. */
async function toggleSessionArchived(session: SessionRow): Promise<void> {
  if (!state.client) return;
  const label = session.title || session.id.slice(0, 8);
  const archived = session.end_reason === "archived";
  try {
    await state.client.setSessionEndReason(session.id, archived ? null : "archived");
    session.end_reason = archived ? undefined : "archived";
    if (state.current?.id === session.id) refreshEndBadge();
    notifySuccess(
      fmt(archived ? t.palette.sessionUnarchived : t.palette.sessionArchived, { label }),
    );
    renderSessions();
  } catch (error) {
    notifyError(fmt(t.palette.sessionArchiveFailed, { error: String(error) }));
  }
}

/** P414: palette action — archive the current session
* (PATCH end_reason=archived; the row stays browseable). */
async function runArchiveSession(): Promise<void> {
  if (!state.client || !state.current) return;
  const label = state.current.title || state.current.id.slice(0, 8);
  try {
    await state.client.setSessionEndReason(state.current.id, "archived");
    state.current.end_reason = "archived";
    refreshEndBadge();
    notifySuccess(fmt(t.palette.sessionArchived, { label }));
    await refreshSessions();
  } catch (error) {
    notifyError(fmt(t.palette.sessionArchiveFailed, { error: String(error) }));
  }
}

/** P414: palette action — unarchive the current session
 * (PATCH end_reason=null clears the recorded end). */
async function runUnarchiveSession(): Promise<void> {
  if (!state.client || !state.current) return;
  const label = state.current.title || state.current.id.slice(0, 8);
  try {
    await state.client.setSessionEndReason(state.current.id, null);
    state.current.end_reason = undefined;
    refreshEndBadge();
    notifySuccess(fmt(t.palette.sessionUnarchived, { label }));
    await refreshSessions();
  } catch (error) {
    notifyError(fmt(t.palette.sessionArchiveFailed, { error: String(error) }));
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
