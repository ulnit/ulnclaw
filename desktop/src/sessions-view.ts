// Sessions view — read-only transcript browser over `/api/sessions` +
// `/api/sessions/:id/messages`: filterable session list on the left,
// full transcript render on the right (role headers, tool-call chips
// with expandable arguments), plus Markdown export of the selected
// session. Complements the chat sidebar, which only renders resumable
// sessions for continued conversation.

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
// P463: transcript tail window before the load-full banner appears.
const TRANSCRIPT_LIMIT = 400;
// P465: messages rendered per batch once the full transcript is loaded.
const RENDER_WINDOW = 200;

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
  private transcriptTotal = 0;
  // P453: project drill-down set from the row chips.
  private projectFilter: string | null = localStorage.getItem(PROJECT_FILTER_KEY);
  // P469: model quick-filter selection.
  private modelFilter: string | null = localStorage.getItem(MODEL_FILTER_KEY);
  // P450: activity-first or title-first list sorting.
  private sortMode: "activity" | "title" =
    localStorage.getItem(SORT_KEY) === "title" ? "title" : "activity";

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
    // P422: bridge back into the chat view (resume the selected session).
    private openInChat: ((session: SessionRow) => void) | null = null,
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
        <button id="sessions-view-prune" class="ghost" data-i18n="sessionsView.prune" data-i18n-title="sessionsView.pruneTitle"></button>
        <button id="sessions-view-archive" class="ghost" data-i18n="sessionsView.archive" data-i18n-title="sessionsView.archiveTitle"></button>
        <button id="sessions-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="sessions-view-status" class="config-status" hidden></div>
      <div class="sessions-view-body">
        <div class="sessions-view-listcol">
          <input id="sessions-view-filter" type="search" data-i18n-ph="sessionsView.filterPlaceholder" />
          <select id="sessions-view-status-filter" data-i18n-title="sessionsView.statusFilterTitle">
            <option value="all" data-i18n="sessionsView.statusAll">All statuses</option>
            <option value="open" data-i18n="sessionsView.statusOpen">Open</option>
            <option value="ended" data-i18n="sessionsView.statusEnded">Ended</option>
            <option value="archived" data-i18n="sessionsView.statusArchived">Archived</option>
          </select>
          <select id="sessions-view-model-filter" data-i18n-title="sessionsView.modelFilterTitle"></select>
          <button id="sessions-view-reason-pill" class="sessions-view-reason-pill" hidden></button>
          <button id="sessions-view-project-pill" class="sessions-view-reason-pill" hidden></button>
          <input id="sessions-view-search" type="search" data-i18n-ph="sessionsView.searchPlaceholder" />
          <div id="sessions-view-list" class="sessions-view-list" tabindex="0"></div>
        </div>
        <div id="sessions-view-transcript" class="sessions-view-transcript" tabindex="0">
          <p class="empty" data-i18n="sessionsView.select"></p>
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
    this.root.querySelector("#sessions-view-filter")!.addEventListener("input", () => {
      this.renderList();
    });
    this.root.querySelector("#sessions-view-status-filter")!.addEventListener("change", () => {
      this.renderList();
    });
    // P469: model quick-filter.
    this.root.querySelector("#sessions-view-model-filter")!.addEventListener("change", () => {
      const select = this.root.querySelector("#sessions-view-model-filter") as HTMLSelectElement;
      this.modelFilter = select.value || null;
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
    });
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
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* on-demand only */
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
      this.all = await client.listSessions();
      this.sortSessions();
      // P446: restore the persisted selection on the first load.
      if (!this.restored) {
        this.restored = true;
        const saved = localStorage.getItem(SELECTED_KEY);
        if (saved && !this.selected && this.all.some((session) => session.id === saved)) {
          this.selected = saved;
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

  /** P448: select a session from outside (command-palette bridge). */
  openSession(sessionId: string): void {
    this.selected = sessionId;
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
      if (this.modelFilter && session.model !== this.modelFilter) return false;
      if (this.projectFilter && session.project !== this.projectFilter) return false;
      if (this.endReasonFilter && session.end_reason !== this.endReasonFilter) return false;
      if (status === "open" && session.end_reason) return false;
      if (status === "ended" && (!session.end_reason || session.end_reason === "archived")) return false;
      if (status === "archived" && session.end_reason !== "archived") return false;
      if (!filter) return true;
      const haystack = `${session.title || ""} ${session.id} ${session.model || ""} ${session.source || ""}`;
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
        let header = "";
        if (showGroups) {
          const group = this.dateGroup(session.last_activity_at || session.started_at);
          if (group !== lastGroup) {
            lastGroup = group;
            header = `<div class="sessions-view-group">${escapeHtml(group)}</div>`;
          }
        }
        return header + `
          <div class="sessions-view-row${active}${kb}" data-id="${escapeHtml(session.id)}">
            <div class="sessions-view-row-title">${escapeHtml(title)}</div>
            <div class="sessions-view-row-meta">
              ${session.model ? `<span class="sessions-view-model">${escapeHtml(session.model)}</span>` : ""}
              ${session.source && session.source !== "gateway" ? `<span class="sessions-view-chip" title="${escapeHtml(session.source)}">${escapeHtml(session.source)}</span>` : ""}
              ${session.end_reason ? `<span class="sessions-view-chip sessions-view-endreason" data-reason="${escapeHtml(session.end_reason)}" title="${escapeHtml(session.end_reason)}">${escapeHtml(session.end_reason)}</span>` : ""}
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
        this.renderList();
        this.loadTranscript(row.dataset.id || "").catch(() => undefined);
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
      this.transcriptTotal = 0;
    }
    pane.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.loading)}</p>`;
    try {
      const messages = await client.messages(sessionId, { limit: TRANSCRIPT_LIMIT });
      if (this.selected !== sessionId) return; // user moved on
      const session = this.all.find((candidate) => candidate.id === sessionId);
      // P465/P468: cache the tail window, count the true total, and render
      // only the most recent batch.
      this.transcriptMessages = messages;
      this.transcriptTotal = session?.message_count || messages.length;
      this.transcriptStart = Math.max(0, messages.length - RENDER_WINDOW);
      const renderedCount = this.transcriptTotal || messages.length;
      const meta = session ? this.renderTranscriptMeta(session, renderedCount) : "";
      pane.innerHTML = meta + this.renderMessages(messages.slice(this.transcriptStart));
      this.updateTranscriptBanner(sessionId);
      // P454: per-message copy actions.
      this.bindCopyButtons(pane);
      pane.scrollTop = 0;
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
   * fetch older windows from the gateway via the `?before=` cursor. */
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
    if (!action) return;
    const banner = document.createElement("div");
    banner.className = "sessions-view-trunc sessions-view-earlier";
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", action);
    banner.appendChild(button);
    pane.insertBefore(banner, pane.querySelector(".sessions-view-msg"));
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
    pane.scrollTop = prevTop + (pane.scrollHeight - prevHeight);
    this.updateTranscriptBanner(sessionId);
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
      const older = await client.messages(sessionId, { before: cursor, limit: TRANSCRIPT_LIMIT });
      if (this.selected !== sessionId) return;
      this.transcriptMessages = [...older, ...this.transcriptMessages];
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      const holder = document.createElement("div");
      holder.innerHTML = this.renderMessages(older);
      this.bindCopyButtons(holder);
      const prevHeight = pane.scrollHeight;
      const prevTop = pane.scrollTop;
      const anchor = pane.querySelector(".sessions-view-msg");
      for (const node of Array.from(holder.children)) pane.insertBefore(node, anchor);
      pane.scrollTop = prevTop + (pane.scrollHeight - prevHeight);
      this.updateTranscriptBanner(sessionId);
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

  private renderTranscriptMeta(session: SessionRow, rendered: number): string {
    const v = t.sessionsView;
    const parts = [
      session.model ? escapeHtml(session.model) : "",
      v.msgCount.replace("{count}", String(rendered)),
      session.project ? `${v.project}: ${escapeHtml(session.project)}` : "",
      session.source ? `${v.source}: ${escapeHtml(session.source)}` : "",
      fmtWhen(session.started_at),
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
        return `
          <div class="sessions-view-msg sessions-view-msg-${escapeHtml(message.role)}">
            <div class="sessions-view-role">${escapeHtml(this.roleLabel(message.role))}${nameTag}${message.content ? `<button class="sessions-view-copy" title="${escapeHtml(t.sessionsView.copyMessageTitle)}">⧉</button>` : ""}</div>
            ${content}
            ${toolCalls}
          </div>`;
      })
      .join("");
  }

  /** P439: open the rename dialog for the selected session. */
  private async renameSelected(): Promise<void> {
    if (!this.client() || !this.selected) return;
    const current = this.all.find((session) => session.id === this.selected);
    this.renameTarget = this.selected;
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

  /** P442: open the delete confirmation dialog for the selected session. */
  private async deleteSelected(): Promise<void> {
    if (!this.client() || !this.selected) return;
    this.deleteTarget = this.selected;
    const current = this.all.find((session) => session.id === this.selected);
    const label = current?.title || this.selected.slice(0, 12);
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
      (this.root.querySelector("#sessions-delete-dialog") as HTMLDialogElement).close();
      this.status(t.sessionsView.deleted.replace("{id}", sessionId.slice(0, 12)), false);
      this.selected = null;
      localStorage.removeItem(SELECTED_KEY);
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      pane.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.select)}</p>`;
      for (const id of ["#sessions-view-export", "#sessions-view-export-html", "#sessions-view-recap", "#sessions-view-fork", "#sessions-view-open-chat", "#sessions-view-delete", "#sessions-view-rename"]) {
        (this.root.querySelector(id) as HTMLButtonElement).hidden = true;
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

  private async forkSelected(): Promise<void> {
    const client = this.client();
    if (!client || !this.selected) return;
    try {
      const forked = await client.forkSession(this.selected);
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
