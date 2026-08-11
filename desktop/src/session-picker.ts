import { ICON } from "./icons";
// Session picker (P254) — dependency-free port of hermes apps/desktop
// `components/session-picker.tsx`: the desktop equivalent of the TUI's
// `/resume` sessions overlay. A focused, type-to-filter list of recent
// sessions that resumes the picked one. In hermes the composer dock and
// the /resume + /sessions + /switch slash commands open it so the
// command feels first-class instead of falling through to the headless
// slash worker; the ulnclaw composer intercepts those same commands.
// P311: queries of 2+ characters additionally fan out to the gateway's
// full-text transcript search (GET /api/sessions/search, FTS5 with LIKE
// fallback) and append snippet rows for sessions whose message bodies
// match — the same store search the Sessions view uses.

import type { SessionRow, SessionSearchHit } from "./gateway";
import { fmt, t } from "./i18n";

export interface SessionPickerHooks {
  sessions(): SessionRow[];
  currentSessionId(): string | null;
  openSession(id: string): void | Promise<void>;
  search?(query: string): Promise<SessionSearchHit[]>;
  /** P550: sidebar pin-state parity — pinned rows float to the top. */
  isPinned?(id: string): boolean;
  togglePin?(session: SessionRow): void;
  /** P557: unread parity — rows with pending replies get the ● mark. */
  isUnread?(id: string): boolean;
}

const SEARCH_DEBOUNCE_MS = 250;
const SNIPPET_MAX = 140;

function normalize(query: string): string {
  return query.trim().toLowerCase();
}

export class SessionPickerDialog {
  private dialog: HTMLDialogElement;
  private search: HTMLInputElement;
  private list: HTMLDivElement;
  private searchTimer: number | null = null;
  private searchSeq = 0;
  private hits: SessionSearchHit[] = [];
  private hitsQuery = "";
  // P478: keyboard-navigation index over the rendered rows.
  private kbIndex = -1;
  private filters: HTMLDivElement;
  private statusFilter: "all" | "open" | "ended" | "archived" = "all";
  private reasonFilter: string | null = null;
  // P558: unread-only quick filter (sidebar P532 parity).
  private unreadOnly = false;

  constructor(private hooks: SessionPickerHooks) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "session-picker-dialog";
    const title = document.createElement("h2");
    title.className = "session-picker-title";
    title.textContent = t.sessionPicker.title;
    this.search = document.createElement("input");
    this.search.type = "text";
    this.search.placeholder = t.sessionPicker.searchPlaceholder;
    this.search.addEventListener("input", () => this.onInput());
    this.search.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        this.dialog.close();
      } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        // P478: up/down cycle the rows.
        event.preventDefault();
        const rows = Array.from(this.list.querySelectorAll<HTMLButtonElement>(".session-picker-row"));
        if (!rows.length) return;
        const delta = event.key === "ArrowDown" ? 1 : -1;
        this.kbIndex = ((this.kbIndex + delta) % rows.length + rows.length) % rows.length;
        this.updateKbHighlight(rows);
      } else if (event.key === "Enter") {
        // P478: Enter opens the highlighted row.
        const rows = Array.from(this.list.querySelectorAll<HTMLButtonElement>(".session-picker-row"));
        if (this.kbIndex >= 0 && rows[this.kbIndex]) {
          event.preventDefault();
          rows[this.kbIndex].click();
        }
      } else if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey
        && (event.key === "p" || event.key === "P")) {
        // P566: Ctrl/Cmd+P toggles the pin on the highlighted row.
        event.preventDefault();
        if (this.kbIndex < 0 || !this.hooks.togglePin) return;
        const rows = Array.from(this.list.querySelectorAll<HTMLButtonElement>(".session-picker-row"));
        const target = rows[this.kbIndex];
        if (!target) return;
        const session = this.hooks.sessions().find((row) => row.id === target.dataset.sessionId);
        if (!session) return;
        const keep = this.kbIndex;
        this.hooks.togglePin(session);
        this.renderList();
        const fresh = Array.from(this.list.querySelectorAll<HTMLButtonElement>(".session-picker-row"));
        if (fresh.length) {
          this.kbIndex = Math.min(keep, fresh.length - 1);
          this.updateKbHighlight(fresh);
        }
      }
    });
    this.filters = document.createElement("div");
    this.filters.className = "session-picker-filters";
    this.list = document.createElement("div");
    this.list.className = "session-picker-list";
    this.dialog.append(title, this.search, this.filters, this.list);
    document.body.appendChild(this.dialog);
  }

  open(): void {
    this.search.value = "";
    this.hits = [];
    this.hitsQuery = "";
    this.kbIndex = -1;
    this.statusFilter = "all";
    this.reasonFilter = null;
    this.unreadOnly = false;
    this.renderList();
    this.dialog.showModal();
    this.search.focus();
  }

  private onInput(): void {
    this.renderList();
    if (this.searchTimer !== null) window.clearTimeout(this.searchTimer);
    const query = this.search.value.trim();
    if (!this.hooks.search || query.length < 2) {
      this.hits = [];
      this.hitsQuery = "";
      return;
    }
    this.searchTimer = window.setTimeout(() => {
      void this.runSearch(query);
    }, SEARCH_DEBOUNCE_MS);
  }

  private async runSearch(query: string): Promise<void> {
    const search = this.hooks.search;
    if (!search) return;
    const seq = ++this.searchSeq;
    try {
      const hits = await search(query);
      if (seq !== this.searchSeq || normalize(this.search.value) !== normalize(query)) return;
      this.hits = hits;
      this.hitsQuery = normalize(query);
      this.renderList();
    } catch {
      // Full-text search is best-effort; local title filtering still works.
    }
  }

  private renderList(): void {
    this.renderFilters();
    this.list.innerHTML = "";
    this.kbIndex = -1;
    const q = normalize(this.search.value);
    const rows = [...this.hooks.sessions()]
      .sort((a, b) => {
        // P550: pinned sessions float to the top (sidebar parity, P548).
        const pinA = this.hooks.isPinned?.(a.id) ? 1 : 0;
        const pinB = this.hooks.isPinned?.(b.id) ? 1 : 0;
        if (pinA !== pinB) return pinB - pinA;
        return b.last_activity_at - a.last_activity_at;
      })
      .filter((session) => this.matchesStatus(session))
      // P558: narrow to sessions waiting on the user.
      .filter((session) => !this.unreadOnly || (this.hooks.isUnread?.(session.id) ?? false))
      .slice(0, 200)
      .filter((session) => {
        if (!q) return true;
        const title = session.title || session.id.slice(0, 8);
        return `${title} ${session.id}`.toLowerCase().includes(q);
      });
    const seen = new Set(rows.map((session) => session.id));
    const filtered = this.statusFilter !== "all" || this.reasonFilter !== null || this.unreadOnly;
    const hits = q.length >= 2 && this.hitsQuery === q && !filtered
      ? this.hits.filter((hit) => !seen.has(hit.session_id))
      : [];
    if (!rows.length && !hits.length) {
      const empty = document.createElement("div");
      empty.className = "session-picker-empty";
      empty.textContent = t.sessionPicker.noResults;
      this.list.appendChild(empty);
      return;
    }
    const activeId = this.hooks.currentSessionId();
    for (const session of rows) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "session-picker-row"
        + (this.hooks.isUnread?.(session.id) ? " unread" : "");
      const icon = document.createElement("span");
      icon.className = "session-picker-icon";
      icon.setAttribute("aria-hidden", "true");
      icon.textContent = "\u{1F4AC}";
      const main = document.createElement("span");
      main.className = "session-picker-main";
      const title = document.createElement("span");
      title.className = "session-picker-row-title";
      title.textContent = session.title || session.id.slice(0, 8);
      main.appendChild(title);
      const bits: string[] = [new Date(session.last_activity_at * 1000).toLocaleString()];
      if (session.message_count != null) {
        bits.push(fmt(t.sessionPicker.messages, { count: session.message_count }));
      }
      // P423: model + end-reason meta, matching the sidebar badges.
      if (session.model) bits.push(session.model);
      if (session.project) bits.push(session.project);
      if (session.end_reason) bits.push(session.end_reason);
      // P629: context-window usage meta (sidebar badge parity, P628).
      if (typeof session.context_percent === "number" && session.context_percent > 0) {
        bits.push(`${session.context_percent}% ctx`);
      }
      const meta = document.createElement("span");
      meta.className = "session-picker-meta";
      meta.textContent = bits.join(" \u00b7 ");
      main.appendChild(meta);
      // P565: last-message snippet when the shell loaded previews.
      const snippet = (session.last_message || "").replace(/\s+/g, " ").trim();
      if (snippet) {
        const snippetEl = document.createElement("span");
        snippetEl.className = "session-picker-snippet";
        snippetEl.textContent =
          snippet.length > SNIPPET_MAX ? `${snippet.slice(0, SNIPPET_MAX)}\u2026` : snippet;
        main.appendChild(snippetEl);
      }
      const check = document.createElement("span");
      check.className = "session-picker-check";
      check.textContent = session.id === activeId ? "\u2713" : "";
      // P550: per-row pin toggle (📌 pinned / 📍 unpinned), mirroring the
      // sidebar hover action.
      const pin = document.createElement("span");
      const rowPinned = this.hooks.isPinned?.(session.id) ?? false;
      pin.className = "session-picker-pin" + (rowPinned ? " pinned" : "");
      pin.title = rowPinned ? t.palette.unpinSession : t.palette.pinSession;
      pin.innerHTML = rowPinned ? ICON.pinFilled : ICON.pin;
      pin.onclick = (event) => {
        event.stopPropagation();
        if (!this.hooks.togglePin) return;
        this.hooks.togglePin(session);
        this.renderList();
      };
      row.append(icon, main, pin, check);
      row.onclick = () => {
        this.dialog.close();
        void this.hooks.openSession(session.id);
      };
      row.dataset.sessionId = session.id;
      this.list.appendChild(row);
    }
    for (const hit of hits) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "session-picker-row";
      const icon = document.createElement("span");
      icon.className = "session-picker-icon";
      icon.setAttribute("aria-hidden", "true");
      icon.textContent = "\u{1F50D}";
      const main = document.createElement("span");
      main.className = "session-picker-main";
      const title = document.createElement("span");
      title.className = "session-picker-row-title";
      title.textContent = hit.title || hit.session_id.slice(0, 8);
      main.appendChild(title);
      const meta = document.createElement("span");
      meta.className = "session-picker-meta";
      const snippet = hit.snippet.replace(/\s+/g, " ").trim();
      meta.textContent = snippet.length > SNIPPET_MAX ? `${snippet.slice(0, SNIPPET_MAX)}\u2026` : snippet;
      main.appendChild(meta);
      const check = document.createElement("span");
      check.className = "session-picker-check";
      check.textContent = hit.session_id === activeId ? "\u2713" : "";
      row.append(icon, main, check);
      row.onclick = () => {
        this.dialog.close();
        void this.hooks.openSession(hit.session_id);
      };
      row.dataset.sessionId = hit.session_id;
      this.list.appendChild(row);
    }
  }

  /** P478: highlight + scroll the keyboard-selected row into view. */
  private updateKbHighlight(rows: HTMLButtonElement[]): void {
    rows.forEach((row, index) => row.classList.toggle("active", index === this.kbIndex));
    if (this.kbIndex >= 0 && rows[this.kbIndex]) {
      rows[this.kbIndex].scrollIntoView({ block: "nearest" });
    }
  }

  // P464: status + end-reason quick-filter chips, mirroring the Sessions
  // view's status semantics (open = no end_reason, ended = non-archived
  // end_reason, archived = end_reason "archived").
  private renderFilters(): void {
    this.filters.innerHTML = "";
    // P558: unread-only chip first, when the shell provides unread state.
    if (this.hooks.isUnread) {
      const unreadChip = document.createElement("button");
      unreadChip.type = "button";
      unreadChip.className = "session-picker-chip" + (this.unreadOnly ? " active" : "");
      unreadChip.textContent = t.sessionPicker.unreadOnly;
      unreadChip.onclick = () => {
        this.unreadOnly = !this.unreadOnly;
        this.renderList();
      };
      this.filters.appendChild(unreadChip);
    }
    const statuses: Array<{ id: "all" | "open" | "ended" | "archived"; label: string }> = [
      { id: "all", label: t.sessionsView.statusAll },
      { id: "open", label: t.sessionsView.statusOpen },
      { id: "ended", label: t.sessionsView.statusEnded },
      { id: "archived", label: t.sessionsView.statusArchived },
    ];
    for (const status of statuses) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "session-picker-chip"
        + (this.reasonFilter === null && this.statusFilter === status.id ? " active" : "");
      chip.textContent = status.label;
      chip.title = t.sessionsView.statusFilterTitle;
      chip.onclick = () => {
        this.statusFilter = status.id;
        this.reasonFilter = null;
        this.renderList();
      };
      this.filters.appendChild(chip);
    }
    const reasons = [...new Set(
      this.hooks.sessions()
        .map((session) => session.end_reason)
        .filter((reason): reason is string => !!reason),
    )].sort();
    if (!reasons.length) return;
    const sep = document.createElement("span");
    sep.className = "session-picker-chip-sep";
    this.filters.appendChild(sep);
    for (const reason of reasons) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "session-picker-chip" + (this.reasonFilter === reason ? " active" : "");
      chip.textContent = reason.replace(/_/g, " ");
      chip.title = fmt(t.session.endReasonTitle, { reason });
      chip.onclick = () => {
        this.reasonFilter = this.reasonFilter === reason ? null : reason;
        this.renderList();
      };
      this.filters.appendChild(chip);
    }
  }

  private matchesStatus(session: SessionRow): boolean {
    if (this.reasonFilter !== null) return session.end_reason === this.reasonFilter;
    if (this.statusFilter === "open") return !session.end_reason;
    if (this.statusFilter === "ended") return !!session.end_reason && session.end_reason !== "archived";
    if (this.statusFilter === "archived") return session.end_reason === "archived";
    return true;
  }
}
