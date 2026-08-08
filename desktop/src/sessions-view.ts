// Sessions view — read-only transcript browser over `/api/sessions` +
// `/api/sessions/:id/messages`: filterable session list on the left,
// full transcript render on the right (role headers, tool-call chips
// with expandable arguments), plus Markdown export of the selected
// session. Complements the chat sidebar, which only renders resumable
// sessions for continued conversation.

import { FindBar } from "./find-bar";
import type { GatewayClient, MessageRow, SessionPruneOptions, SessionRow } from "./gateway";
import { fmt, onLocaleChange, t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fmtWhen(ts: number | null | undefined): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString();
}

/** P567: compact session duration (45s / 12m / 3h05m / 2d4h). */
function formatDurationCompact(totalSecs: number): string {
  if (totalSecs < 60) return `${totalSecs}s`;
  const minutes = Math.floor(totalSecs / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h${String(minutes % 60).padStart(2, "0")}m`;
  return `${Math.floor(hours / 24)}d${hours % 24}h`;
}

// P440: persistence keys for the list filters.
const STATUS_FILTER_KEY = "ulnclaw.sessions.statusFilter";
const REASON_FILTER_KEY = "ulnclaw.sessions.reasonFilter";
// P446: persistence key for the selected session.
const SELECTED_KEY = "ulnclaw.sessions.selected";
// P449: persistence key for the transcript search text.
const SEARCH_KEY = "ulnclaw.sessions.search";
// P450: persistence key for the list sort mode.
const SORT_KEY = "ulnclaw.sessions.sort";
// P453: persistence key for the project drill-down.
const PROJECT_FILTER_KEY = "ulnclaw.sessions.projectFilter";
// P469: persisted model quick-filter selection.
const MODEL_FILTER_KEY = "ulnclaw.sessions.modelFilter";
// P481: persisted source quick-filter selection.
const SOURCE_FILTER_KEY = "ulnclaw.sessions.sourceFilter";
// P511: persisted set of hidden transcript roles.
const ROLE_VISIBILITY_KEY = "ulnclaw.sessions.hiddenRoles";
// P463: transcript tail window before the load-full banner appears.
const TRANSCRIPT_LIMIT = 400;
// P465: messages rendered per batch once the full transcript is loaded.
const RENDER_WINDOW = 200;
// P508: hard cap on rendered transcript messages — the pane keeps a
// sliding window over the cached transcript so very long expanded
// transcripts never grow the DOM without bound.
const MAX_RENDERED = 1200;

export class SessionsViewWidget {
  private all: SessionRow[] = [];
  private selected: string | null = null;
  private pruneMode: "prune" | "archive" = "prune";
  // P438: end-reason drill-down set from the row chips.
  private endReasonFilter: string | null = null;
  // P439: target of the rename dialog while it is open.
  private renameTarget: string | null = null;
  // P442: target of the delete dialog while it is open.
  private deleteTarget: string | null = null;
  // P473: ids this UI deleted (suppresses cross-client toasts).
  private localDeletes = new Set<string>();
  // P443: keyboard navigation state for the list.
  private visible: SessionRow[] = [];
  private kbIndex = 0;
  // P446: whether the persisted selection has been restored this mount.
  private restored = false;
  // P463: per-selection transcript pagination state.
  private transcriptSession: string | null = null;
  // P465/P468: cached contiguous tail of the transcript, the index of the
  // first rendered message, and the server-side total message count.
  private transcriptMessages: MessageRow[] = [];
  private transcriptStart = 0;
  // P508: exclusive end index of the rendered window in transcriptMessages.
  private transcriptEnd = 0;
  private transcriptTotal = 0;
  // P453: project drill-down set from the row chips.
  private projectFilter: string | null = localStorage.getItem(PROJECT_FILTER_KEY);
  // P469: model quick-filter selection.
  private modelFilter: string | null = localStorage.getItem(MODEL_FILTER_KEY);
  // P481: source quick-filter selection.
  private sourceFilter: string | null = localStorage.getItem(SOURCE_FILTER_KEY);
  // P511: transcript roles currently hidden from view.
  private hiddenRoles = new Set<string>(
    (localStorage.getItem(ROLE_VISIBILITY_KEY) || "").split(",").filter(Boolean),
  );
  // P470: find-in-transcript bar scoped to this view.
  private findBar: FindBar | null = null;
  private active = false;
  // P471: background poll timer + whether the user expanded past the tail.
  private pollTimer: number | null = null;
  private transcriptExpanded = false;
  // P493: debounce timer for session.message catch-ups.
  private messageEventTimer: number | null = null;
  // P450: activity-first or title-first list sorting.
  private sortMode: "activity" | "title" =
    localStorage.getItem(SORT_KEY) === "title" ? "title" : "activity";

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
    // P422: bridge back into the chat view (resume the selected session).
    private openInChat: ((session: SessionRow) => void) | null = null,
    // P496: unread tracking hooks (read-state lives in the shell).
    private isUnread: ((sessionId: string) => boolean) | null = null,
    private onRead: ((sessionId: string) => void) | null = null,
    // P556: pin hooks — the pin set itself lives in the shell (P548).
    private isPinned: ((sessionId: string) => boolean) | null = null,
    private onTogglePin: ((session: SessionRow) => void) | null = null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="sessions-view-header">
        <span id="sessions-view-count" class="jobs-counts"></span>
        <span id="sessions-view-stats" class="config-note" hidden></span>
        <span class="spacer"></span>
        <button id="sessions-view-rename" class="ghost" data-i18n-title="sessionsView.renameTitle" hidden>✎</button>
        <button id="sessions-view-delete" class="ghost" data-i18n-title="sessionsView.deleteTitle" hidden>🗑</button>
        <button id="sessions-view-fork" class="ghost" data-i18n-title="sessionsView.forkTitle" hidden>⑂</button>
        <button id="sessions-view-open-chat" class="ghost" data-i18n="sessionsView.openInChat" data-i18n-title="sessionsView.openInChatTitle" hidden></button>
        <button id="sessions-view-recap" class="ghost" data-i18n="sessionsView.recap" data-i18n-title="sessionsView.recapTitle" hidden></button>
        <button id="sessions-view-export" class="ghost" data-i18n-title="sessionsView.exportTitle" hidden>⭳ MD</button>
        <button id="sessions-view-export-html" class="ghost" data-i18n-title="sessionsView.exportHtmlTitle" hidden>⭳ HTML</button>
        <button id="sessions-view-export-json" class="ghost" data-i18n-title="sessionsView.exportJsonTitle" hidden>⭳ JSON</button>
        <button id="sessions-view-import" class="ghost" data-i18n="sessionsView.import" data-i18n-title="sessionsView.importTitle"></button>
        <button id="sessions-view-list-csv" class="ghost" data-i18n="sessionsView.listCsv" data-i18n-title="sessionsView.listCsvTitle"></button>
        <button id="sessions-view-prune" class="ghost" data-i18n="sessionsView.prune" data-i18n-title="sessionsView.pruneTitle"></button>
        <button id="sessions-view-archive" class="ghost" data-i18n="sessionsView.archive" data-i18n-title="sessionsView.archiveTitle"></button>
        <select id="sessions-view-dayjump" class="ghost" data-i18n-title="session.dayJumpTitle" hidden></select>
        <button id="sessions-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="sessions-view-status" class="config-status" hidden></div>
      <div class="sessions-view-body">
        <div class="sessions-view-listcol">
          <input id="sessions-view-filter" type="search" data-i18n-ph="sessionsView.filterPlaceholder" />
          <label class="check sessions-view-unread-only"><input id="sessions-view-unread-only" type="checkbox" /><span data-i18n="sessionsView.unreadOnly">Unread only</span></label>
          <select id="sessions-view-status-filter" data-i18n-title="sessionsView.statusFilterTitle">
            <option value="all" data-i18n="sessionsView.statusAll">All statuses</option>
            <option value="open" data-i18n="sessionsView.statusOpen">Open</option>
            <option value="ended" data-i18n="sessionsView.statusEnded">Ended</option>
            <option value="archived" data-i18n="sessionsView.statusArchived">Archived</option>
          </select>
          <select id="sessions-view-model-filter" data-i18n-title="sessionsView.modelFilterTitle"></select>
          <select id="sessions-view-source-filter" data-i18n-title="sessionsView.sourceFilterTitle"></select>
          <button id="sessions-view-reason-pill" class="sessions-view-reason-pill" hidden></button>
          <button id="sessions-view-project-pill" class="sessions-view-reason-pill" hidden></button>
          <input id="sessions-view-search" type="search" data-i18n-ph="sessionsView.searchPlaceholder" />
          <button id="sessions-view-clear-filters" class="ghost" data-i18n="sessionsView.clearFilters" hidden></button>
          <div id="sessions-view-list" class="sessions-view-list" tabindex="0"></div>
        </div>
        <div class="sessions-view-transcriptcol">
          <div id="sessions-view-rolebar" class="sessions-view-rolebar">
            <span class="sessions-view-rolebar-label" data-i18n="sessionsView.roleShow">Show:</span>
            <button class="sessions-view-roletoggle" data-role="user" data-i18n="sessionsView.roleUser">user</button>
            <button class="sessions-view-roletoggle" data-role="assistant" data-i18n="sessionsView.roleAssistant">assistant</button>
            <button class="sessions-view-roletoggle" data-role="tool" data-i18n="sessionsView.roleTool">tool</button>
            <button class="sessions-view-roletoggle" data-role="system" data-i18n="sessionsView.roleSystem">system</button>
          </div>
          <div id="sessions-view-transcript" class="sessions-view-transcript" tabindex="0">
            <p class="empty" data-i18n="sessionsView.select"></p>
          </div>
        </div>
      </div>
      <dialog id="sessions-rename-dialog">
        <h2 data-i18n="sessionsView.renameTitle">Rename this session</h2>
        <label><span data-i18n="sessionsView.renamePrompt">New title (empty clears):</span>
          <input id="sessions-rename-input" type="text" />
        </label>
        <p id="sessions-rename-status" class="config-note" hidden></p>
        <menu>
          <button id="sessions-rename-cancel" class="ghost" data-i18n="chrome.cancel">Cancel</button>
          <button id="sessions-rename-save" value="default" data-i18n="chrome.save">Save</button>
        </menu>
      </dialog>
      <dialog id="sessions-delete-dialog">
        <h2 data-i18n="sessionsView.deleteTitle">Delete this session</h2>
        <p id="sessions-delete-message"></p>
        <menu>
          <button id="sessions-delete-cancel" class="ghost" data-i18n="chrome.cancel">Cancel</button>
          <button id="sessions-delete-confirm" class="danger" value="default" data-i18n="chrome.delete">Delete</button>
        </menu>
      </dialog>
      <dialog id="sessions-prune-dialog">
        <h2 id="sessions-prune-title"></h2>
        <label><span data-i18n="sessionsView.olderThanLabel">Last activity older than</span>
          <input id="sessions-prune-older" type="text" placeholder="90d / 2026-01-01" />
        </label>
        <label><span data-i18n="sessionsView.sourceLabel">Source filter (optional)</span>
          <input id="sessions-prune-source" type="text" placeholder="cli / cron / gateway" />
        </label>
        <label class="check" id="sessions-prune-archived-row">
          <input id="sessions-prune-include-archived" type="checkbox" />
          <span data-i18n="sessionsView.includeArchived">Include already-archived sessions</span>
        </label>
        <p id="sessions-prune-status" class="config-note"></p>
        <menu>
          <button id="sessions-prune-cancel" class="ghost" data-i18n="chrome.cancel">Cancel</button>
          <button id="sessions-prune-preview" class="ghost" data-i18n="sessionsView.preview">Preview</button>
          <button id="sessions-prune-apply" class="primary" data-i18n="sessionsView.apply">Apply</button>
        </menu>
      </dialog>
    `;
    this.root.querySelector("#sessions-view-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    // P511: transcript role-visibility toggles.
    this.root.querySelectorAll<HTMLElement>(".sessions-view-roletoggle").forEach((button) => {
      button.addEventListener("click", () => {
        const role = button.dataset.role || "";
        if (!role) return;
        if (this.hiddenRoles.has(role)) this.hiddenRoles.delete(role);
        else this.hiddenRoles.add(role);
        localStorage.setItem(ROLE_VISIBILITY_KEY, Array.from(this.hiddenRoles).join(","));
        this.applyRoleVisibility();
      });
    });
    this.applyRoleVisibility();
    this.root.querySelector("#sessions-view-filter")!.addEventListener("input", () => {
      this.renderList();
    });
    // P499: unread-only quick toggle.
    this.root.querySelector("#sessions-view-unread-only")!.addEventListener("change", () => {
      this.renderList();
    });
    this.root.querySelector("#sessions-view-status-filter")!.addEventListener("change", () => {
      this.renderList();
    });
    // P479: date-jump dropdown over the transcript's day dividers.
    this.root.querySelector("#sessions-view-dayjump")!.addEventListener("change", () => {
      const select = this.root.querySelector("#sessions-view-dayjump") as HTMLSelectElement;
      const day = select.value;
      select.value = "";
      if (!day) return;
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      pane.querySelector<HTMLElement>(`.day-divider[data-day="${day}"]`)?.scrollIntoView({
        block: "start",
        behavior: "smooth",
      });
    });
    // P469: model quick-filter.
    this.root.querySelector("#sessions-view-model-filter")!.addEventListener("change", () => {
      const select = this.root.querySelector("#sessions-view-model-filter") as HTMLSelectElement;
      this.modelFilter = select.value || null;
      this.renderList();
    });
    // P481: source quick-filter.
    this.root.querySelector("#sessions-view-source-filter")!.addEventListener("change", () => {
      const select = this.root.querySelector("#sessions-view-source-filter") as HTMLSelectElement;
      this.sourceFilter = select.value || null;
      this.renderList();
    });
    // P436: option labels are driven by updateStatusOptions (live counts);
    // strip data-i18n so applyStatic doesn't fight the count suffixes.
    const statusSelect = this.root.querySelector("#sessions-view-status-filter") as HTMLSelectElement;
    for (const option of Array.from(statusSelect.options)) option.removeAttribute("data-i18n");
    // P440: restore persisted filter state.
    const savedStatus = localStorage.getItem(STATUS_FILTER_KEY);
    if (savedStatus && ["all", "open", "ended", "archived"].includes(savedStatus)) {
      statusSelect.value = savedStatus;
    }
    this.endReasonFilter = localStorage.getItem(REASON_FILTER_KEY);
    this.updateStatusOptions();
    onLocaleChange(() => {
      this.updateStatusOptions();
      this.updateModelOptions();
      this.updateSourceOptions();
      this.findBar?.rerender();
    });
    // P470: Ctrl/Cmd+F find-in-transcript while this view is shown.
    this.findBar = new FindBar(
      this.root,
      this.root.querySelector("#sessions-view-transcript") as HTMLElement,
      () => this.active,
    );
    this.root.querySelector("#sessions-view-reason-pill")!.addEventListener("click", () => {
      this.endReasonFilter = null;
      this.renderList();
    });
    this.root.querySelector("#sessions-view-project-pill")!.addEventListener("click", () => {
      this.projectFilter = null;
      this.renderList();
    });
    // P455: Home/End + PageUp/PageDown paging in the transcript pane.
    (this.root.querySelector("#sessions-view-transcript") as HTMLElement).addEventListener("keydown", (event) => {
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      if (event.key === "Home") {
        event.preventDefault();
        pane.scrollTop = 0;
      } else if (event.key === "End") {
        event.preventDefault();
        pane.scrollTop = pane.scrollHeight;
      } else if (event.key === "PageUp") {
        event.preventDefault();
        pane.scrollTop -= pane.clientHeight * 0.9;
      } else if (event.key === "PageDown") {
        event.preventDefault();
        pane.scrollTop += pane.clientHeight * 0.9;
      } else if (
        event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.shiftKey &&
        (event.key === "ArrowUp" || event.key === "ArrowDown")
      ) {
        // P480: step between day dividers.
        event.preventDefault();
        this.jumpDayDivider(event.key === "ArrowDown" ? 1 : -1);
      }
    });
    // P483: type-to-focus — printable keys while the list has focus jump
    // into the filter box (chat type-to-focus parity).
    (this.root.querySelector("#sessions-view-list") as HTMLElement).addEventListener("keydown", (event) => {
      if (
        event.key.length === 1 &&
        event.key !== "Enter" &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey
      ) {
        event.preventDefault();
        const filterInput = this.root.querySelector("#sessions-view-filter") as HTMLInputElement;
        filterInput.focus();
        filterInput.value += event.key;
        filterInput.dispatchEvent(new Event("input", { bubbles: true }));
      }
    });
    // P450: the count header toggles activity ↔ title sorting.
    const countHeader = this.root.querySelector("#sessions-view-count") as HTMLElement;
    countHeader.classList.add("clickable");
    countHeader.addEventListener("click", () => {
      this.sortMode = this.sortMode === "title" ? "activity" : "title";
      localStorage.setItem(SORT_KEY, this.sortMode);
      this.applySortLabel();
      this.sortSessions();
      this.renderList();
    });
    this.applySortLabel();
    let searchDebounce: number | null = null;
    // P492: download the filtered list as CSV.
    this.root.querySelector("#sessions-view-list-csv")!.addEventListener("click", () => {
      this.exportListCsv();
    });
    // P482: one-click reset of every list filter.
    this.root.querySelector("#sessions-view-clear-filters")!.addEventListener("click", () => {
      (this.root.querySelector("#sessions-view-filter") as HTMLInputElement).value = "";
      (this.root.querySelector("#sessions-view-unread-only") as HTMLInputElement).checked = false;
      const searchInput = this.root.querySelector("#sessions-view-search") as HTMLInputElement;
      searchInput.value = "";
      localStorage.removeItem(SEARCH_KEY);
      (this.root.querySelector("#sessions-view-status-filter") as HTMLSelectElement).value = "all";
      this.modelFilter = null;
      this.sourceFilter = null;
      this.endReasonFilter = null;
      this.projectFilter = null;
      this.renderList();
    });
    this.root.querySelector("#sessions-view-search")!.addEventListener("input", () => {
      if (searchDebounce !== null) window.clearTimeout(searchDebounce);
      searchDebounce = window.setTimeout(() => {
        this.runSearch().catch(() => undefined);
      }, 350);
    });
    // P449: persist the search text and restore it (re-running if set).
    const searchInput = this.root.querySelector("#sessions-view-search") as HTMLInputElement;
    searchInput.value = localStorage.getItem(SEARCH_KEY) || "";
    searchInput.addEventListener("input", () => {
      localStorage.setItem(SEARCH_KEY, searchInput.value);
    });
    if (searchInput.value.trim()) {
      this.runSearch().catch(() => undefined);
    }
    this.root.querySelector("#sessions-view-export")!.addEventListener("click", () => {
      this.exportSelected("md");
    });
    this.root.querySelector("#sessions-view-export-html")!.addEventListener("click", () => {
      this.exportSelected("html");
    });
    this.root.querySelector("#sessions-view-export-json")!.addEventListener("click", () => {
      this.exportSelected("json");
    });
    this.root.querySelector("#sessions-view-import")!.addEventListener("click", () => {
      this.importSessions();
    });
    this.root.querySelector("#sessions-view-recap")!.addEventListener("click", () => {
      this.toggleRecap().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-fork")!.addEventListener("click", () => {
      this.forkSelected().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-open-chat")!.addEventListener("click", () => {
      this.resumeSelected();
    });
    this.root.querySelector("#sessions-view-delete")!.addEventListener("click", () => {
      this.deleteSelected().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-rename")!.addEventListener("click", () => {
      this.renameSelected().catch(() => undefined);
    });
    this.root.querySelector("#sessions-rename-save")!.addEventListener("click", () => {
      void this.commitRename();
    });
    this.root.querySelector("#sessions-rename-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#sessions-rename-dialog") as HTMLDialogElement).close();
    });
    (this.root.querySelector("#sessions-rename-input") as HTMLInputElement).addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        void this.commitRename();
      }
    });
    this.root.querySelector("#sessions-delete-confirm")!.addEventListener("click", () => {
      void this.commitDelete();
    });
    this.root.querySelector("#sessions-delete-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#sessions-delete-dialog") as HTMLDialogElement).close();
    });
    // P443: ↑/↓ + Enter keyboard navigation over the visible rows.
    (this.root.querySelector("#sessions-view-list") as HTMLElement).addEventListener("keydown", (event) => {
      if (this.visible.length === 0) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        this.kbIndex = Math.min(this.kbIndex + 1, this.visible.length - 1);
        this.renderList();
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        this.kbIndex = Math.max(this.kbIndex - 1, 0);
        this.renderList();
      } else if (event.key === "Enter") {
        event.preventDefault();
        const session = this.visible[this.kbIndex];
        if (session) {
          this.selected = session.id;
          this.onRead?.(session.id);
          this.renderList();
          this.loadTranscript(session.id).catch(() => undefined);
        }
      } else {
        return;
      }
      this.root.querySelector(".sessions-view-row.kb-focus")?.scrollIntoView({ block: "nearest" });
    });
    this.root.querySelector("#sessions-view-prune")!.addEventListener("click", () => {
      this.openPruneDialog("prune");
    });
    this.root.querySelector("#sessions-view-archive")!.addEventListener("click", () => {
      this.openPruneDialog("archive");
    });
    this.root.querySelector("#sessions-prune-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#sessions-prune-dialog") as HTMLDialogElement).close();
    });
    this.root.querySelector("#sessions-prune-preview")!.addEventListener("click", () => {
      this.prunePreview().catch(() => undefined);
    });
    this.root.querySelector("#sessions-prune-apply")!.addEventListener("click", () => {
      this.pruneApply().catch(() => undefined);
    });
  }

  start(): void {
    this.active = true;
    this.refresh().catch(() => undefined);
    // P471: light background polling while the view is shown.
    if (this.pollTimer === null) {
      this.pollTimer = window.setInterval(() => this.poll(), 10_000);
    }
  }

  stop(): void {
    // P470: leaving the view closes any open find bar.
    this.active = false;
    this.findBar?.close();
    if (this.pollTimer !== null) {
      window.clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
    if (this.messageEventTimer !== null) {
      window.clearTimeout(this.messageEventTimer);
      this.messageEventTimer = null;
    }
  }

  /** P471: keep the list and the open transcript fresh in the background. */
  private poll(): void {
    if (!this.active || document.hidden) return;
    this.refresh().catch(() => undefined);
    void this.pollTranscriptTotal();
  }

  private async pollTranscriptTotal(): Promise<void> {
    const client = this.client();
    const sessionId = this.transcriptSession;
    if (!client || !sessionId || sessionId !== this.selected) return;
    try {
      const total = await client.messagesTotal(sessionId);
      if (this.selected !== sessionId || total === this.transcriptTotal) return;
      if (total < this.transcriptTotal) {
        // The transcript shrank (compression/prune) — reload from scratch.
        this.loadTranscript(sessionId).catch(() => undefined);
        return;
      }
      if (!this.transcriptExpanded) {
        // P476: append just the new messages over the `after` cursor.
        const fresh = await client.messages(sessionId, {
          timestamps: true,
          after: this.transcriptTotal,
        });
        if (this.selected !== sessionId) return;
        if (fresh.length !== total - this.transcriptTotal) {
          this.loadTranscript(sessionId).catch(() => undefined);
          return;
        }
        this.transcriptMessages.push(...fresh);
        this.transcriptTotal = total;
        const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
        const holder = document.createElement("div");
        holder.innerHTML = this.renderMessages(fresh);
        this.bindCopyButtons(holder);
        for (const node of Array.from(holder.children)) pane.appendChild(node);
        this.transcriptEnd = this.transcriptMessages.length;
        this.trimWindow(pane, "head");
        this.syncDayDividers(pane);
      this.updateDayJump(pane);
        const session = this.all.find((candidate) => candidate.id === sessionId);
        const existing = pane.querySelector(".sessions-view-meta");
        if (session && existing) {
          existing.outerHTML = this.renderTranscriptMeta(session, total);
        }
        this.findBar?.refresh();
      } else {
        // The user is reading history — only update the count + banner.
        this.transcriptTotal = total;
        const session = this.all.find((candidate) => candidate.id === sessionId);
        const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
        const existing = pane?.querySelector(".sessions-view-meta");
        if (session && existing) {
          existing.outerHTML = this.renderTranscriptMeta(session, total);
        }
        this.updateTranscriptBanner(sessionId);
      }
    } catch {
      // Probe is best-effort; the next poll retries.
    }
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#sessions-view-status") as HTMLElement;
    el.hidden = !message;
    el.textContent = message;
    el.classList.toggle("error", isError);
  }

  async refresh(): Promise<void> {
    const client = this.client();
    if (!client) {
      this.status(t.config.notConnected, true);
      return;
    }
    try {
      // P510: request last-message previews for the list rows.
      this.all = await client.listSessions(true);
      this.sortSessions();
      // P446: restore the persisted selection on the first load.
      if (!this.restored) {
        this.restored = true;
        const saved = localStorage.getItem(SELECTED_KEY);
        if (saved && !this.selected && this.all.some((session) => session.id === saved)) {
          this.selected = saved;
          this.onRead?.(saved);
          this.loadTranscript(saved).catch(() => undefined);
        }
      }
      (this.root.querySelector("#sessions-view-count") as HTMLElement).textContent =
        t.sessionsView.count.replace("{count}", String(this.all.length));
      this.renderList();
      this.status("");
      this.loadStats().catch(() => undefined);
    } catch (error) {
      this.status(
        t.sessionsView.loadFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** P451: date group label for a row timestamp (activity sort only). */
  private dateGroup(ts: number | null | undefined): string {
    if (!ts) return t.session.groupOlder;
    const now = new Date();
    const startOfToday =
      new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000;
    if (ts >= startOfToday) return t.session.dayToday;
    if (ts >= startOfToday - 86400) return t.session.groupYesterday;
    if (ts >= startOfToday - 7 * 86400) return t.session.groupWeek;
    return t.session.groupOlder;
  }

  /** P450: sort the loaded rows per the active sort mode. */
  private sortSessions(): void {
    if (this.sortMode === "title") {
      this.all.sort((a, b) => (a.title || a.id).localeCompare(b.title || b.id));
    } else {
      this.all.sort(
        (a, b) => (b.last_activity_at || b.started_at) - (a.last_activity_at || a.started_at),
      );
    }
    // P556: pinned sessions float to the top regardless of sort mode
    // (stable sort keeps the intra-group ordering intact).
    if (this.isPinned) {
      const pinned = this.isPinned;
      this.all.sort(
        (a, b) => (pinned(b.id) ? 1 : 0) - (pinned(a.id) ? 1 : 0),
      );
    }
  }

  /** P450: count-header tooltip reflects the active sort mode. */
  private applySortLabel(): void {
    const countHeader = this.root.querySelector("#sessions-view-count") as HTMLElement;
    countHeader.title =
      this.sortMode === "title" ? t.sessionsView.sortByTitle : t.sessionsView.sortByActivity;
  }

  /** P469: rebuild the model filter options from the loaded sessions,
   * keeping the selection when the model still exists. */
  private updateModelOptions(): void {
    const select = this.root.querySelector("#sessions-view-model-filter") as HTMLSelectElement;
    if (!select) return;
    const models = [...new Set(
      this.all.map((session) => session.model).filter((model): model is string => !!model),
    )].sort();
    if (this.modelFilter && !models.includes(this.modelFilter)) this.modelFilter = null;
    select.innerHTML = "";
    const all = document.createElement("option");
    all.value = "";
    all.textContent = t.sessionsView.modelAll;
    select.appendChild(all);
    for (const model of models) {
      const option = document.createElement("option");
      option.value = model;
      option.textContent = model;
      select.appendChild(option);
    }
    select.value = this.modelFilter ?? "";
  }

  /** P492: download the currently filtered session list as CSV. */
  private exportListCsv(): void {
    const rows = this.visible.length > 0 ? this.visible : this.all;
    const header = [
      "id", "title", "source", "model", "project", "message_count",
      "end_reason", "started_at", "last_activity_at",
    ];
    const lines = [header.join(",")];
    for (const session of rows) {
      lines.push([
        this.csvCell(session.id),
        this.csvCell(session.title),
        this.csvCell(session.source),
        this.csvCell(session.model),
        this.csvCell(session.project ?? null),
        this.csvCell(session.message_count ?? null),
        this.csvCell(session.end_reason ?? null),
        this.csvCell(session.started_at),
        this.csvCell(session.last_activity_at),
      ].join(","));
    }
    const blob = new Blob([lines.join("\n")], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `ulnclaw-sessions-${Date.now()}.csv`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 2000);
    this.status(fmt(t.sessionsView.listCsvDone, { count: String(rows.length) }), false);
  }

  private csvCell(value: string | number | null): string {
    const text = String(value ?? "");
    return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
  }

  /** P493: the gateway appended a message — catch up soon (debounced so
   * streaming bursts coalesce into one probe + append pass). */
  notifyMessageAppended(sessionId: string): void {
    if (sessionId !== this.selected) return;
    if (this.messageEventTimer !== null) window.clearTimeout(this.messageEventTimer);
    this.messageEventTimer = window.setTimeout(() => {
      this.messageEventTimer = null;
      void this.pollTranscriptTotal();
    }, 400);
  }

  /** P490: run a full-text search from outside (composer /search). */
  searchFor(query: string): void {
    const input = this.root.querySelector("#sessions-view-search") as HTMLInputElement;
    if (!input) return;
    input.value = query;
    localStorage.setItem(SEARCH_KEY, query);
    void this.runSearch();
  }

  /** P473: true (and consumes) when this id was deleted from this UI. */
  takeLocalDelete(sessionId: string): boolean {
    if (this.localDeletes.has(sessionId)) {
      this.localDeletes.delete(sessionId);
      return true;
    }
    return false;
  }

  /** P481: rebuild the source filter options from the loaded sessions. */
  private updateSourceOptions(): void {
    const select = this.root.querySelector("#sessions-view-source-filter") as HTMLSelectElement;
    if (!select) return;
    const sources = [...new Set(
      this.all.map((session) => session.source).filter((source): source is string => !!source),
    )].sort();
    if (this.sourceFilter && !sources.includes(this.sourceFilter)) this.sourceFilter = null;
    select.innerHTML = "";
    const all = document.createElement("option");
    all.value = "";
    all.textContent = t.sessionsView.sourceAll;
    select.appendChild(all);
    for (const source of sources) {
      const option = document.createElement("option");
      option.value = source;
      option.textContent = source;
      select.appendChild(option);
    }
    select.value = this.sourceFilter ?? "";
  }

  /** P496: the currently selected session id (shell unread tracking). */
  selectedId(): string | null {
    return this.selected;
  }

  /** P448: select a session from outside (command-palette bridge). */
  openSession(sessionId: string): void {
    this.selected = sessionId;
    this.onRead?.(sessionId);
    this.kbIndex = Math.max(0, this.visible.findIndex((session) => session.id === sessionId));
    this.renderList();
    this.loadTranscript(sessionId).catch(() => undefined);
  }

  /** P467: reload the transcript in place when a run settles in the
   * selected session (keeps the current tail-window vs full mode). */
  refreshTranscript(sessionId: string): void {
    if (this.selected !== sessionId) return;
    this.loadTranscript(sessionId).catch(() => undefined);
  }

  /** Prune/archive dialog (P314) — mirrors `ulnclaw sessions prune`
   * and `sessions archive` over POST /api/sessions/prune|archive. */
  private openPruneDialog(mode: "prune" | "archive"): void {
    this.pruneMode = mode;
    const title = this.root.querySelector("#sessions-prune-title") as HTMLElement;
    title.textContent =
      mode === "prune" ? t.sessionsView.pruneDialogTitle : t.sessionsView.archiveDialogTitle;
    (this.root.querySelector("#sessions-prune-archived-row") as HTMLElement).hidden =
      mode === "archive";
    (this.root.querySelector("#sessions-prune-status") as HTMLElement).textContent = "";
    (this.root.querySelector("#sessions-prune-dialog") as HTMLDialogElement).showModal();
  }

  private pruneOptions(dryRun: boolean): SessionPruneOptions {
    const older = (this.root.querySelector("#sessions-prune-older") as HTMLInputElement).value.trim();
    const source = (this.root.querySelector("#sessions-prune-source") as HTMLInputElement).value.trim();
    const includeArchived =
      this.pruneMode === "prune" &&
      (this.root.querySelector("#sessions-prune-include-archived") as HTMLInputElement).checked;
    return {
      older_than: older || undefined,
      source: source || undefined,
      include_archived: includeArchived || undefined,
      dry_run: dryRun || undefined,
    };
  }

  private async prunePreview(): Promise<void> {
    const client = this.client();
    const statusEl = this.root.querySelector("#sessions-prune-status") as HTMLElement;
    if (!client) return;
    try {
      const result =
        this.pruneMode === "prune"
          ? await client.sessionsPrune(this.pruneOptions(true))
          : await client.sessionsArchive(this.pruneOptions(true));
      const count = result.count ?? 0;
      statusEl.textContent =
        count === 0
          ? t.sessionsView.previewEmpty
          : t.sessionsView.previewCount.replace("{count}", String(count));
    } catch (error) {
      statusEl.textContent = t.sessionsView.failed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private async pruneApply(): Promise<void> {
    const client = this.client();
    const statusEl = this.root.querySelector("#sessions-prune-status") as HTMLElement;
    if (!client) return;
    try {
      const preview =
        this.pruneMode === "prune"
          ? await client.sessionsPrune(this.pruneOptions(true))
          : await client.sessionsArchive(this.pruneOptions(true));
      const count = preview.count ?? 0;
      if (count === 0) {
        statusEl.textContent = t.sessionsView.previewEmpty;
        return;
      }
      const confirmText = (
        this.pruneMode === "prune"
          ? t.sessionsView.confirmPrune
          : t.sessionsView.confirmArchive
      ).replace("{count}", String(count));
      if (!window.confirm(confirmText)) return;
      const result =
        this.pruneMode === "prune"
          ? await client.sessionsPrune(this.pruneOptions(false))
          : await client.sessionsArchive(this.pruneOptions(false));
      (this.root.querySelector("#sessions-prune-dialog") as HTMLDialogElement).close();
      await this.refresh();
      const affected = result.affected ?? 0;
      this.status(
        this.pruneMode === "prune"
          ? t.sessionsView.appliedPruned.replace("{count}", String(affected))
          : t.sessionsView.appliedArchived.replace("{count}", String(affected)),
      );
    } catch (error) {
      statusEl.textContent = t.sessionsView.failed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** Store census from /api/storage (P312): sessions/messages/disk. */
  private async loadStats(): Promise<void> {
    const client = this.client();
    const el = this.root.querySelector("#sessions-view-stats") as HTMLElement;
    if (!client) {
      el.hidden = true;
      return;
    }
    try {
      const stats = await client.storageStats();
      el.textContent = t.sessionsView.stats
        .replace("{sessions}", String(stats.sessions))
        .replace("{messages}", String(stats.messages))
        .replace("{size}", this.fmtBytes(stats.size_bytes + stats.wal_bytes));
      el.hidden = false;
    } catch {
      el.hidden = true;
    }
  }

  private fmtBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  private async runSearch(): Promise<void> {
    const client = this.client();
    const list = this.root.querySelector("#sessions-view-list") as HTMLElement;
    const query = (this.root.querySelector("#sessions-view-search") as HTMLInputElement).value.trim();
    if (!query) {
      this.renderList();
      return;
    }
    if (!client) return;
    try {
      const hits = await client.searchSessions(query);
      if (hits.length === 0) {
        list.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.noResults)}</p>`;
        return;
      }
      list.innerHTML = hits
        .map((hit) => {
          const title = hit.title || hit.session_id.slice(0, 8);
          return `
            <div class="sessions-view-row" data-id="${escapeHtml(hit.session_id)}">
              <div class="sessions-view-row-title">${escapeHtml(title)}</div>
              <div class="sessions-view-snippet">${escapeHtml(hit.snippet)}</div>
            </div>`;
        })
        .join("");
      for (const row of Array.from(list.querySelectorAll<HTMLElement>(".sessions-view-row"))) {
        row.addEventListener("click", () => {
          this.selected = row.dataset.id || null;
          (this.root.querySelector("#sessions-view-search") as HTMLInputElement).value = "";
          this.renderList();
          this.loadTranscript(row.dataset.id || "").catch(() => undefined);
        });
      }
    } catch (error) {
      list.innerHTML = `<p class="empty">${escapeHtml(
        t.sessionsView.searchFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      )}</p>`;
    }
  }

  private renderList(): void {
    const list = this.root.querySelector("#sessions-view-list") as HTMLElement;
    // P446: persist the current selection for the next visit.
    if (this.selected) {
      localStorage.setItem(SELECTED_KEY, this.selected);
    }
    const filter = (this.root.querySelector("#sessions-view-filter") as HTMLInputElement).value
      .trim()
      .toLowerCase();
    const status = (this.root.querySelector("#sessions-view-status-filter") as HTMLSelectElement).value;
    // P440: persist the status filter selection.
    localStorage.setItem(STATUS_FILTER_KEY, status);
    // P436: live per-status counts on the filter options.
    this.updateStatusOptions({
      all: this.all.length,
      open: this.all.filter((s) => !s.end_reason).length,
      ended: this.all.filter((s) => s.end_reason && s.end_reason !== "archived").length,
      archived: this.all.filter((s) => s.end_reason === "archived").length,
    });
    // P469: model quick-filter options + persistence.
    this.updateModelOptions();
    if (this.modelFilter) localStorage.setItem(MODEL_FILTER_KEY, this.modelFilter);
    else localStorage.removeItem(MODEL_FILTER_KEY);
    // P481: source quick-filter options + persistence.
    this.updateSourceOptions();
    if (this.sourceFilter) localStorage.setItem(SOURCE_FILTER_KEY, this.sourceFilter);
    else localStorage.removeItem(SOURCE_FILTER_KEY);
    // P499: unread-only toggle state.
    const unreadOnly = (this.root.querySelector("#sessions-view-unread-only") as HTMLInputElement).checked;
    // P482: show the clear-filters action while any filter is active.
    const searchValue = (this.root.querySelector("#sessions-view-search") as HTMLInputElement).value.trim();
    const filtersActive = Boolean(filter) || status !== "all" || this.modelFilter !== null
      || this.sourceFilter !== null || this.endReasonFilter !== null || this.projectFilter !== null
      || Boolean(searchValue) || unreadOnly;
    (this.root.querySelector("#sessions-view-clear-filters") as HTMLButtonElement).hidden = !filtersActive;
    this.visible = [];
    // P438: show/clear the end-reason drill-down pill.
    const reasonPill = this.root.querySelector("#sessions-view-reason-pill") as HTMLButtonElement;
    if (this.endReasonFilter) {
      reasonPill.hidden = false;
      reasonPill.textContent = `${fmt(t.sessionsView.reasonFilter, { reason: this.endReasonFilter })} ✕`;
      localStorage.setItem(REASON_FILTER_KEY, this.endReasonFilter);
    } else {
      reasonPill.hidden = true;
      localStorage.removeItem(REASON_FILTER_KEY);
    }
    // P453: show/clear the project drill-down pill.
    const projectPill = this.root.querySelector("#sessions-view-project-pill") as HTMLButtonElement;
    if (this.projectFilter) {
      projectPill.hidden = false;
      projectPill.textContent = `${fmt(t.sessionsView.projectFilter, { project: this.projectFilter })} ✕`;
      localStorage.setItem(PROJECT_FILTER_KEY, this.projectFilter);
    } else {
      projectPill.hidden = true;
      localStorage.removeItem(PROJECT_FILTER_KEY);
    }
    const rows = this.all.filter((session) => {
      if (unreadOnly && !this.isUnread?.(session.id)) return false;
      if (this.sourceFilter && session.source !== this.sourceFilter) return false;
      if (this.modelFilter && session.model !== this.modelFilter) return false;
      if (this.projectFilter && session.project !== this.projectFilter) return false;
      if (this.endReasonFilter && session.end_reason !== this.endReasonFilter) return false;
      if (status === "open" && session.end_reason) return false;
      if (status === "ended" && (!session.end_reason || session.end_reason === "archived")) return false;
      if (status === "archived" && session.end_reason !== "archived") return false;
      if (!filter) return true;
      const haystack = `${session.title || ""} ${session.id} ${session.model || ""} ${session.source || ""} ${session.last_message || ""}`;
      return haystack.toLowerCase().includes(filter);
    });
    if (rows.length === 0) {
      list.innerHTML = `<p class="empty">${escapeHtml(filter ? t.sessionsView.noMatch : t.sessionsView.empty)}</p>`;
      return;
    }
    this.visible = rows;
    if (this.kbIndex >= rows.length) this.kbIndex = Math.max(0, rows.length - 1);
    // P451: date group headers while sorted by activity.
    const showGroups = this.sortMode === "activity";
    let lastGroup = "";
    list.innerHTML = rows
      .map((session, index) => {
        const title = session.title || session.id.slice(0, 8);
        const active = session.id === this.selected ? " active" : "";
        const kb = index === this.kbIndex ? " kb-focus" : "";
        const unread = this.isUnread?.(session.id) ? " unread" : "";
        const pinned = this.isPinned?.(session.id) === true;
        let header = "";
        if (showGroups) {
          const group = this.dateGroup(session.last_activity_at || session.started_at);
          if (group !== lastGroup) {
            lastGroup = group;
            header = `<div class="sessions-view-group">${escapeHtml(group)}</div>`;
          }
        }
        return header + `
          <div class="sessions-view-row${active}${kb}${unread}" data-id="${escapeHtml(session.id)}">
            <span class="sessions-view-row-actions">
              <button class="sessions-view-row-action" data-action="pin" title="${escapeHtml(pinned ? t.palette.unpinSession : t.palette.pinSession)}">${pinned ? "\u{1F4CC}" : "\u{1F4CD}"}</button>
              <button class="sessions-view-row-action" data-action="rename" title="${escapeHtml(t.sessionsView.renameTitle)}">✎</button>
              <button class="sessions-view-row-action" data-action="retitle" title="${escapeHtml(t.sessionsView.retitleTitle)}">✨</button>
              <button class="sessions-view-row-action" data-action="fork" title="${escapeHtml(t.sessionsView.forkTitle)}">⑂</button>
              <button class="sessions-view-row-action" data-action="delete" title="${escapeHtml(t.sessionsView.deleteTitle)}">🗑</button>
            </span>
            <div class="sessions-view-row-title">${pinned ? "\u{1F4CC} " : ""}${escapeHtml(title)}</div>
            ${session.last_message ? `<div class="sessions-view-row-snippet">${escapeHtml(session.last_message)}</div>` : ""}
            <div class="sessions-view-row-meta">
              ${session.source && session.source !== "gateway" ? `<span class="sessions-view-chip sessions-view-source" data-source="${escapeHtml(session.source)}" title="${escapeHtml(session.source)}">${escapeHtml(session.source)}</span>` : ""}
              ${session.end_reason ? `<span class="sessions-view-chip sessions-view-endreason" data-reason="${escapeHtml(session.end_reason)}" title="${escapeHtml(session.end_reason)}">${escapeHtml(session.end_reason)}</span>` : ""}
              ${session.archived ? `<span class="sessions-view-chip sessions-view-endreason" title="${escapeHtml(t.sessionsView.statusArchived)}">␡ ${escapeHtml(t.sessionsView.statusArchived)}</span>` : ""}
              ${session.model ? `<span class="sessions-view-chip sessions-view-model" data-model="${escapeHtml(session.model)}" title="${escapeHtml(session.model)}">${escapeHtml(session.model)}</span>` : ""}
              ${session.project ? `<span class="sessions-view-chip sessions-view-project" data-project="${escapeHtml(session.project)}" title="${escapeHtml(session.project)}">${escapeHtml(session.project)}</span>` : ""}
              ${session.message_count ? `<span class="sessions-view-msgcount">${escapeHtml(fmt(t.sessionsView.msgCount, { count: String(session.message_count) }))}</span>` : ""}
              <span title="${escapeHtml(fmtWhen(session.started_at))}">${fmtWhen(session.last_activity_at || session.started_at)}</span>
            </div>
          </div>`;
      })
      .join("");
    for (const row of Array.from(list.querySelectorAll<HTMLElement>(".sessions-view-row"))) {
      row.addEventListener("click", () => {
        this.selected = row.dataset.id || null;
        this.onRead?.(row.dataset.id || "");
        this.renderList();
        this.loadTranscript(row.dataset.id || "").catch(() => undefined);
      });
      // P487: double-click resumes the session in the chat view.
      row.addEventListener("dblclick", () => {
        const session = this.all.find((candidate) => candidate.id === row.dataset.id);
        if (session && this.openInChat) this.openInChat(session);
      });
    }
    // P489: per-row hover quick actions (rename / fork / delete).
    for (const button of Array.from(list.querySelectorAll<HTMLButtonElement>(".sessions-view-row-action"))) {
      button.addEventListener("click", (event) => {
        event.stopPropagation();
        const id = button.closest(".sessions-view-row")?.getAttribute("data-id") || "";
        if (!id) return;
        if (button.dataset.action === "rename") this.renameSelected(id).catch(() => undefined);
        else if (button.dataset.action === "retitle") this.retitleSelected(id).catch(() => undefined);
        else if (button.dataset.action === "fork") this.forkSelected(id).catch(() => undefined);
        else if (button.dataset.action === "delete") this.deleteSelected(id).catch(() => undefined);
        else if (button.dataset.action === "pin") {
          // P556: toggle the pin, then re-sort + re-render in place.
          const session = this.all.find((candidate) => candidate.id === id);
          if (session && this.onTogglePin) {
            this.onTogglePin(session);
            this.sortSessions();
            this.renderList();
          }
        }
      });
    }
    // P438: end-reason chips drill the list down to that reason.
    for (const chip of Array.from(list.querySelectorAll<HTMLElement>(".sessions-view-endreason"))) {
      chip.addEventListener("click", (event) => {
        event.stopPropagation();
        this.endReasonFilter = chip.dataset.reason || null;
        this.renderList();
      });
    }
    // P488: non-gateway source chips drill the list down to that source.
    for (const chip of Array.from(list.querySelectorAll<HTMLElement>(".sessions-view-source"))) {
      chip.addEventListener("click", (event) => {
        event.stopPropagation();
        this.sourceFilter = chip.dataset.source || null;
        this.renderList();
      });
    }
    // P486: model chips drill the list down to that model.
    for (const chip of Array.from(list.querySelectorAll<HTMLElement>(".sessions-view-model"))) {
      chip.addEventListener("click", (event) => {
        event.stopPropagation();
        this.modelFilter = chip.dataset.model || null;
        this.renderList();
      });
    }
    // P453: project chips drill the list down to that project.
    for (const chip of Array.from(list.querySelectorAll<HTMLElement>(".sessions-view-project"))) {
      chip.addEventListener("click", (event) => {
        event.stopPropagation();
        this.projectFilter = chip.dataset.project || null;
        this.renderList();
      });
    }
  }

  /** P436: status filter option labels, optionally suffixed with counts. */
  private updateStatusOptions(counts?: { all: number; open: number; ended: number; archived: number }): void {
    const select = this.root.querySelector("#sessions-view-status-filter") as HTMLSelectElement;
    if (!select) return;
    const labels: Record<string, string> = {
      all: t.sessionsView.statusAll,
      open: t.sessionsView.statusOpen,
      ended: t.sessionsView.statusEnded,
      archived: t.sessionsView.statusArchived,
    };
    for (const option of Array.from(select.options)) {
      const base = labels[option.value] ?? option.value;
      const count = counts?.[option.value as keyof typeof counts];
      option.textContent = count === undefined ? base : `${base} (${count})`;
    }
  }

  private async loadTranscript(sessionId: string): Promise<void> {
    const client = this.client();
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    const exportBtn = this.root.querySelector("#sessions-view-export") as HTMLButtonElement;
    if (!client) return;
    // P463: reset the pagination window when the selection changes.
    if (this.transcriptSession !== sessionId) {
      this.transcriptSession = sessionId;
      this.transcriptMessages = [];
      this.transcriptStart = 0;
      this.transcriptEnd = 0;
      this.transcriptTotal = 0;
      this.transcriptExpanded = false;
    }
    pane.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.loading)}</p>`;
    try {
      const messages = await client.messages(sessionId, { timestamps: true, limit: TRANSCRIPT_LIMIT });
      if (this.selected !== sessionId) return; // user moved on
      const session = this.all.find((candidate) => candidate.id === sessionId);
      // P465/P468: cache the tail window, count the true total, and render
      // only the most recent batch.
      this.transcriptMessages = messages;
      this.transcriptTotal = session?.message_count || messages.length;
      this.transcriptStart = Math.max(0, messages.length - RENDER_WINDOW);
      this.transcriptEnd = messages.length;
      const renderedCount = this.transcriptTotal || messages.length;
      const meta = session ? this.renderTranscriptMeta(session, renderedCount) : "";
      pane.innerHTML = meta + this.renderMessages(messages.slice(this.transcriptStart));
      this.updateTranscriptBanner(sessionId);
      this.syncDayDividers(pane);
      this.updateDayJump(pane);
      // P454: per-message copy actions.
      this.bindCopyButtons(pane);
      pane.scrollTop = 0;
      this.findBar?.refresh();
      exportBtn.hidden = false;
      (this.root.querySelector("#sessions-view-export-html") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-export-json") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-recap") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-fork") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-open-chat") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-delete") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-rename") as HTMLButtonElement).hidden = false;
    } catch (error) {
      if (this.selected !== sessionId) return;
      pane.innerHTML = `<p class="empty">${escapeHtml(
        t.sessionsView.transcriptFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      )}</p>`;
      exportBtn.hidden = true;
      (this.root.querySelector("#sessions-view-dayjump") as HTMLSelectElement).hidden = true;
      (this.root.querySelector("#sessions-view-export-html") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-export-json") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-recap") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-fork") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-open-chat") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-delete") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-rename") as HTMLButtonElement).hidden = true;
    }
  }

  /** P465/P468: single top banner — reveal cached messages first, then
   * fetch older windows from the gateway via the `?before=` cursor.
   * P508: plus a bottom "show newer" banner while the rendered window
   * is trimmed at the tail. */
  private updateTranscriptBanner(sessionId: string): void {
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    pane.querySelector(".sessions-view-trunc")?.remove();
    let label = "";
    let action: (() => void) | null = null;
    if (this.transcriptStart > 0) {
      label = fmt(t.sessionsView.showEarlier, { count: String(this.transcriptStart) });
      action = () => this.revealEarlier(sessionId);
    } else if (this.transcriptMessages.length < this.transcriptTotal) {
      const remaining = this.transcriptTotal - this.transcriptMessages.length;
      label = fmt(t.sessionsView.loadEarlier, { count: String(remaining) });
      action = () => void this.loadEarlier(sessionId);
    }
    if (action) {
      const banner = document.createElement("div");
      banner.className = "sessions-view-trunc sessions-view-earlier";
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.addEventListener("click", action);
      banner.appendChild(button);
      pane.insertBefore(banner, pane.querySelector(".sessions-view-msg"));
    }
    const hiddenNewer = this.transcriptMessages.length - this.transcriptEnd;
    if (hiddenNewer > 0) {
      const banner = document.createElement("div");
      banner.className = "sessions-view-trunc sessions-view-newer";
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = fmt(t.sessionsView.showNewer, { count: String(hiddenNewer) });
      button.addEventListener("click", () => this.revealLater(sessionId));
      banner.appendChild(button);
      pane.appendChild(banner);
    }
  }

  /**
   * P508: enforce MAX_RENDERED by trimming the rendered window from the
   * opposite side of a reveal. Head trims compensate scrollTop so the
   * viewport stays put; callers re-sync day dividers/jump/find state.
   */
  private trimWindow(pane: HTMLElement, side: "head" | "tail"): void {
    const messages = Array.from(pane.querySelectorAll<HTMLElement>(".sessions-view-msg"));
    let excess = messages.length - MAX_RENDERED;
    if (excess <= 0) return;
    if (side === "tail") {
      for (let index = messages.length - 1; index >= 0 && excess > 0; index--, excess--) {
        messages[index].remove();
        this.transcriptEnd -= 1;
      }
      return;
    }
    for (let index = 0; index < messages.length && excess > 0; index++, excess--) {
      const height = messages[index].offsetHeight;
      messages[index].remove();
      this.transcriptStart += 1;
      pane.scrollTop -= height;
    }
  }

  /** P508: re-render the next cached window at the bottom of the pane. */
  private revealLater(sessionId: string): void {
    if (this.selected !== sessionId) return;
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    const start = this.transcriptEnd;
    const end = Math.min(this.transcriptMessages.length, start + RENDER_WINDOW);
    if (end <= start) return;
    const holder = document.createElement("div");
    holder.innerHTML = this.renderMessages(this.transcriptMessages.slice(start, end));
    this.bindCopyButtons(holder);
    for (const node of Array.from(holder.children)) pane.appendChild(node);
    this.transcriptEnd = end;
    this.trimWindow(pane, "head");
    this.updateTranscriptBanner(sessionId);
    this.syncDayDividers(pane);
    this.updateDayJump(pane);
    this.findBar?.refresh();
  }

  private revealEarlier(sessionId: string): void {
    if (this.selected !== sessionId) return;
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    const end = this.transcriptStart;
    const start = Math.max(0, end - RENDER_WINDOW);
    if (start >= end) return;
    const holder = document.createElement("div");
    holder.innerHTML = this.renderMessages(this.transcriptMessages.slice(start, end));
    this.bindCopyButtons(holder);
    const prevHeight = pane.scrollHeight;
    const prevTop = pane.scrollTop;
    const anchor = pane.querySelector(".sessions-view-msg");
    for (const node of Array.from(holder.children)) pane.insertBefore(node, anchor);
    this.transcriptStart = start;
    this.transcriptExpanded = true;
    pane.scrollTop = prevTop + (pane.scrollHeight - prevHeight);
    this.trimWindow(pane, "tail");
    this.updateTranscriptBanner(sessionId);
    this.syncDayDividers(pane);
      this.updateDayJump(pane);
    this.findBar?.refresh();
  }

  /** P468: fetch the previous window from the gateway and prepend it. */
  private async loadEarlier(sessionId: string): Promise<void> {
    const client = this.client();
    if (!client || this.selected !== sessionId) return;
    const cursor = this.transcriptTotal - this.transcriptMessages.length;
    if (cursor <= 0) return;
    const button = this.root.querySelector<HTMLButtonElement>(".sessions-view-earlier button");
    if (button) button.disabled = true;
    try {
      const older = await client.messages(sessionId, { timestamps: true, before: cursor, limit: TRANSCRIPT_LIMIT });
      if (this.selected !== sessionId) return;
      this.transcriptMessages = [...older, ...this.transcriptMessages];
      // P508: everything cached is now rendered until the tail trim runs.
      this.transcriptEnd = this.transcriptMessages.length;
      this.transcriptExpanded = true;
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      const holder = document.createElement("div");
      holder.innerHTML = this.renderMessages(older);
      this.bindCopyButtons(holder);
      const prevHeight = pane.scrollHeight;
      const prevTop = pane.scrollTop;
      const anchor = pane.querySelector(".sessions-view-msg");
      for (const node of Array.from(holder.children)) pane.insertBefore(node, anchor);
      pane.scrollTop = prevTop + (pane.scrollHeight - prevHeight);
      this.trimWindow(pane, "tail");
      this.updateTranscriptBanner(sessionId);
      this.syncDayDividers(pane);
      this.updateDayJump(pane);
      this.findBar?.refresh();
    } catch (error) {
      this.status(
        t.sessionsView.transcriptFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
      if (button) button.disabled = false;
    }
  }

  /** P454/P465: per-message copy actions over rendered transcript nodes. */
  private bindCopyButtons(scope: ParentNode): void {
    scope.querySelectorAll<HTMLElement>(".sessions-view-copy").forEach((button) => {
      button.addEventListener("click", () => {
        const content = button
          .closest(".sessions-view-msg")
          ?.querySelector(".sessions-view-content");
        const text = content?.textContent ?? "";
        void navigator.clipboard.writeText(text).then(
          () => this.status(t.sessionsView.copied, false),
          () => this.status(t.sessionsView.copyFailed, true),
        );
      });
    });
  }

  /** P480: step between day dividers in the transcript pane. */
  private jumpDayDivider(direction: 1 | -1): void {
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    const dividers = Array.from(pane.querySelectorAll<HTMLElement>(".day-divider"));
    if (!dividers.length) return;
    const paneTop = pane.getBoundingClientRect().top;
    let target: HTMLElement | null = null;
    if (direction === 1) {
      target = dividers.find((divider) => divider.getBoundingClientRect().top - paneTop > 4) ?? null;
    } else {
      for (let i = dividers.length - 1; i >= 0; i--) {
        if (dividers[i].getBoundingClientRect().top - paneTop < -4) {
          target = dividers[i];
          break;
        }
      }
      target = target ?? dividers[0];
    }
    if (!target) return;
    pane.scrollTop += target.getBoundingClientRect().top - paneTop - 8;
  }

  /** P479: local calendar-day key for divider lookup. */
  private dayKey(timestamp: number): string {
    const date = new Date(timestamp * 1000);
    const month = String(date.getMonth() + 1).padStart(2, "0");
    const day = String(date.getDate()).padStart(2, "0");
    return `${date.getFullYear()}-${month}-${day}`;
  }

  /** P479: populate the date-jump dropdown from the rendered dividers. */
  private updateDayJump(pane: HTMLElement): void {
    const select = this.root.querySelector("#sessions-view-dayjump") as HTMLSelectElement;
    if (!select) return;
    const dividers = Array.from(pane.querySelectorAll<HTMLElement>(".day-divider"));
    if (dividers.length < 2) {
      select.hidden = true;
      select.innerHTML = "";
      return;
    }
    const current = select.value;
    select.innerHTML = "";
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = t.session.dayJumpTitle;
    select.appendChild(placeholder);
    for (const divider of dividers) {
      const option = document.createElement("option");
      option.value = divider.dataset.day || "";
      option.textContent = divider.querySelector(".day-divider-label")?.textContent ?? option.value;
      select.appendChild(option);
    }
    select.value = dividers.some((divider) => divider.dataset.day === current) ? current : "";
    select.hidden = false;
  }

  /** P477: localized calendar-day label for transcript day dividers. */
  private dayLabel(timestamp: number): string {
    const date = new Date(timestamp * 1000);
    const now = new Date();
    if (
      date.getFullYear() === now.getFullYear() &&
      date.getMonth() === now.getMonth() &&
      date.getDate() === now.getDate()
    ) {
      return t.session.dayToday;
    }
    return date.toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
  }

  /** P477: ensure one day divider between messages on different calendar
   * days — re-run after any render/prepend/append pass. */
  /** P511: mirror persisted role visibility onto the pane + toggles. */
  private applyRoleVisibility(): void {
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    for (const role of ["user", "assistant", "tool", "system"]) {
      pane.classList.toggle(`hide-role-${role}`, this.hiddenRoles.has(role));
    }
    this.root.querySelectorAll<HTMLElement>(".sessions-view-roletoggle").forEach((button) => {
      const role = button.dataset.role || "";
      const active = role !== "" && !this.hiddenRoles.has(role);
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  private syncDayDividers(pane: HTMLElement): void {
    pane.querySelectorAll(".day-divider").forEach((divider) => divider.remove());
    const messages = Array.from(pane.querySelectorAll<HTMLElement>(".sessions-view-msg"));
    let prev: HTMLElement | null = null;
    for (const msg of messages) {
      const ts = msg.dataset.ts ? Number(msg.dataset.ts) : Number.NaN;
      if (prev !== null && Number.isFinite(ts)) {
        const prevTs = prev.dataset.ts ? Number(prev.dataset.ts) : Number.NaN;
        if (
          Number.isFinite(prevTs) &&
          new Date(prevTs * 1000).toDateString() !== new Date(ts * 1000).toDateString()
        ) {
          const divider = document.createElement("div");
          divider.className = "day-divider";
          divider.dataset.day = this.dayKey(ts);
          const label = document.createElement("span");
          label.className = "day-divider-label";
          label.textContent = this.dayLabel(ts);
          divider.appendChild(label);
          msg.before(divider);
        }
      }
      if (Number.isFinite(ts)) prev = msg;
    }
  }

  private renderTranscriptMeta(session: SessionRow, rendered: number): string {
    const v = t.sessionsView;
    const durationSecs = Math.max(
      0,
      Math.floor((session.last_activity_at || session.started_at) - session.started_at),
    );
    const parts = [
      session.model ? escapeHtml(session.model) : "",
      v.msgCount.replace("{count}", String(rendered)),
      session.project ? `${v.project}: ${escapeHtml(session.project)}` : "",
      session.source ? `${v.source}: ${escapeHtml(session.source)}` : "",
      // P522: end-reason + archived markers in the transcript meta strip.
      session.end_reason ? `${v.metaEnded}: ${escapeHtml(session.end_reason)}` : "",
      session.archived ? `\u2421 ${escapeHtml(v.statusArchived)}` : "",
      fmtWhen(session.started_at),
      // P567: how long the session ran (started → last activity).
      durationSecs > 0 ? `${v.metaDuration}: ${formatDurationCompact(durationSecs)}` : "",
    ].filter(Boolean);
    return `<div class="sessions-view-meta">${parts.join(" · ")}</div>`;
  }

  private roleLabel(role: string): string {
    const v = t.sessionsView;
    switch (role) {
      case "user":
        return v.roleUser;
      case "assistant":
        return v.roleAssistant;
      case "tool":
        return v.roleTool;
      case "system":
        return v.roleSystem;
      default:
        return role;
    }
  }

  private renderMessages(messages: MessageRow[]): string {
    if (messages.length === 0) {
      return `<p class="empty">${escapeHtml(t.sessionsView.emptyTranscript)}</p>`;
    }
    return messages
      .map((message) => {
        const toolCalls = (message.tool_calls || [])
          .map((call) => {
            const name = call.function?.name || "tool";
            const args = call.function?.arguments || "";
            return `
              <details class="sessions-view-toolcall">
                <summary>🔧 ${escapeHtml(name)}</summary>
                <pre>${escapeHtml(args)}</pre>
              </details>`;
          })
          .join("");
        const content = message.content
          ? `<div class="sessions-view-content">${escapeHtml(message.content)}</div>`
          : "";
        const nameTag = message.name ? ` · ${escapeHtml(message.name)}` : "";
        if (!content && !toolCalls) return "";
        // P475: per-message stored timestamp as a hover tooltip.
        const whenAttr = message.timestamp
          ? ` title="${escapeHtml(new Date(message.timestamp * 1000).toLocaleString())}"`
          : "";
        const tsAttr = message.timestamp ? ` data-ts="${message.timestamp}"` : "";
        return `
          <div class="sessions-view-msg sessions-view-msg-${escapeHtml(message.role)}"${tsAttr}>
            <div class="sessions-view-role"${whenAttr}>${escapeHtml(this.roleLabel(message.role))}${nameTag}${message.content ? `<button class="sessions-view-copy" title="${escapeHtml(t.sessionsView.copyMessageTitle)}">⧉</button>` : ""}</div>
            ${content}
            ${toolCalls}
          </div>`;
      })
      .join("");
  }

  /** P439: open the rename dialog for the selected session. */
  private async renameSelected(sessionId?: string): Promise<void> {
    const target = sessionId ?? this.selected;
    if (!this.client() || !target) return;
    const current = this.all.find((session) => session.id === target);
    this.renameTarget = target;
    const input = this.root.querySelector("#sessions-rename-input") as HTMLInputElement;
    input.value = current?.title || "";
    const statusEl = this.root.querySelector("#sessions-rename-status") as HTMLElement;
    statusEl.hidden = true;
    statusEl.textContent = "";
    (this.root.querySelector("#sessions-rename-dialog") as HTMLDialogElement).showModal();
    input.focus();
    input.select();
  }

  /** P439: commit the rename dialog over PATCH /api/sessions/:id. */
  private async commitRename(): Promise<void> {
    const client = this.client();
    const sessionId = this.renameTarget;
    if (!client || !sessionId) return;
    const input = this.root.querySelector("#sessions-rename-input") as HTMLInputElement;
    const statusEl = this.root.querySelector("#sessions-rename-status") as HTMLElement;
    try {
      await client.renameSession(sessionId, input.value.trim());
      (this.root.querySelector("#sessions-rename-dialog") as HTMLDialogElement).close();
      this.status(t.sessionsView.renamed, false);
      await this.refresh();
    } catch (error) {
      statusEl.hidden = false;
      statusEl.textContent = t.sessionsView.renameFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** P568: regenerate the hovered session's title via the LLM (P559). */
  private async retitleSelected(id: string): Promise<void> {
    const client = this.client();
    const session = this.all.find((row) => row.id === id);
    if (!client || !session) return;
    try {
      const result = await client.retitleSession(id, true);
      if (!result || result.status === "rejected") {
        this.status(t.sessionsView.retitleFailed, true);
        return;
      }
      if (result.status === "unchanged") {
        this.status(t.sessionsView.retitleUnchanged, false);
        return;
      }
      session.title = result.new_title;
      this.sortSessions();
      this.renderList();
      this.status(fmt(t.sessionsView.retitleApplied, { label: result.new_title }), false);
    } catch {
      this.status(t.sessionsView.retitleFailed, true);
    }
  }

  /** P442: open the delete confirmation dialog for the selected session. */
  private async deleteSelected(sessionId?: string): Promise<void> {
    const target = sessionId ?? this.selected;
    if (!this.client() || !target) return;
    this.deleteTarget = target;
    const current = this.all.find((session) => session.id === target);
    const label = current?.title || target.slice(0, 12);
    const message = this.root.querySelector("#sessions-delete-message") as HTMLElement;
    message.textContent = t.sessionsView.deleteConfirm.replace("{id}", label);
    (this.root.querySelector("#sessions-delete-dialog") as HTMLDialogElement).showModal();
  }

  /** P442: commit the confirmed delete over DELETE /api/sessions/:id. */
  private async commitDelete(): Promise<void> {
    const client = this.client();
    const sessionId = this.deleteTarget;
    if (!client || !sessionId) return;
    try {
      await client.deleteSession(sessionId);
      this.localDeletes.add(sessionId);
      (this.root.querySelector("#sessions-delete-dialog") as HTMLDialogElement).close();
      this.status(t.sessionsView.deleted.replace("{id}", sessionId.slice(0, 12)), false);
      this.selected = null;
      localStorage.removeItem(SELECTED_KEY);
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      pane.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.select)}</p>`;
      for (const id of ["#sessions-view-export", "#sessions-view-export-html", "#sessions-view-recap", "#sessions-view-fork", "#sessions-view-open-chat", "#sessions-view-delete", "#sessions-view-rename", "#sessions-view-dayjump"]) {
        (this.root.querySelector(id) as HTMLElement).hidden = true;
      }
      await this.refresh();
    } catch (error) {
      (this.root.querySelector("#sessions-delete-dialog") as HTMLDialogElement).close();
      this.status(
        t.sessionsView.deleteFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** P422: hand the selected session back to the chat view. */
  private resumeSelected(): void {
    if (!this.selected || !this.openInChat) return;
    const session = this.all.find((candidate) => candidate.id === this.selected);
    if (session) this.openInChat(session);
  }

  private async forkSelected(sessionId?: string): Promise<void> {
    const client = this.client();
    const target = sessionId ?? this.selected;
    if (!client || !target) return;
    try {
      const forked = await client.forkSession(target);
      this.status(
        t.sessionsView.forked.replace("{id}", forked.id.slice(0, 12)),
        false,
      );
      await this.refresh();
      this.selected = forked.id;
      this.renderList();
      this.loadTranscript(forked.id).catch(() => undefined);
    } catch (error) {
      this.status(
        t.sessionsView.forkFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async toggleRecap(): Promise<void> {
    const client = this.client();
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    if (!client || !this.selected) return;
    const existing = pane.querySelector("#sessions-view-recap-panel") as HTMLElement | null;
    if (existing) {
      existing.remove();
      return;
    }
    const panel = document.createElement("details");
    panel.id = "sessions-view-recap-panel";
    panel.open = true;
    panel.innerHTML = `<summary>${escapeHtml(t.sessionsView.recap)}</summary>
      <pre class="sessions-view-recap-body">${escapeHtml(t.sessionsView.loading)}</pre>`;
    pane.insertBefore(panel, pane.firstChild);
    try {
      const recap = await client.sessionRecap(this.selected);
      panel.querySelector("pre")!.textContent = recap || t.sessionsView.emptyTranscript;
    } catch (error) {
      panel.querySelector("pre")!.textContent = t.sessionsView.recapFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private exportSelected(format: "md" | "html" | "json"): void {
    const client = this.client();
    if (!client || !this.selected) return;
    const sessionId = this.selected;
    client
      .exportSession(sessionId, format)
      .then(({ blob, filename }) => {
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = filename;
        document.body.appendChild(link);
        link.click();
        link.remove();
        window.setTimeout(() => URL.revokeObjectURL(url), 5_000);
      })
      .catch(() => {
        this.status(t.sessionsView.exportFailed, true);
      });
  }

  /** P348: import portable session JSON (own ⭳ JSON exports or hermes
   * dashboard exports) over POST /api/sessions/import. */
  private importSessions(): void {
    const client = this.client();
    if (!client) return;
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,application/json";
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return;
      void this.importSessionFile(client, file);
    };
    input.click();
  }

  private async importSessionFile(client: GatewayClient, file: File): Promise<void> {
    try {
      const parsed: unknown = JSON.parse(await file.text());
      const sessions = Array.isArray(parsed)
        ? parsed
        : (parsed as { sessions?: unknown } | null)?.sessions;
      if (!Array.isArray(sessions)) {
        this.status(t.sessionsView.importParseFailed, true);
        return;
      }
      const result = await client.sessionsImport(sessions);
      const summary = t.sessionsView.imported
        .replace("{imported}", String(result.imported))
        .replace("{messages}", String(result.messages))
        .replace("{skipped}", String(result.skipped));
      this.status(
        result.ok ? summary : `${summary} — ${result.errors[0]?.error ?? ""}`,
        !result.ok,
      );
      await this.refresh();
    } catch (error) {
      this.status(
        t.sessionsView.importFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }
}
