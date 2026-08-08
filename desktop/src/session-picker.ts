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
      }
    });
    this.list = document.createElement("div");
    this.list.className = "session-picker-list";
    this.dialog.append(title, this.search, this.list);
    document.body.appendChild(this.dialog);
  }

  open(): void {
    this.search.value = "";
    this.hits = [];
    this.hitsQuery = "";
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
    this.list.innerHTML = "";
    const q = normalize(this.search.value);
    const rows = [...this.hooks.sessions()]
      .sort((a, b) => b.last_activity_at - a.last_activity_at)
      .slice(0, 200)
      .filter((session) => {
        if (!q) return true;
        const title = session.title || session.id.slice(0, 8);
        return `${title} ${session.id}`.toLowerCase().includes(q);
      });
    const seen = new Set(rows.map((session) => session.id));
    const hits = q.length >= 2 && this.hitsQuery === q
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
      row.className = "session-picker-row";
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
      const meta = document.createElement("span");
      meta.className = "session-picker-meta";
      meta.textContent = bits.join(" \u00b7 ");
      main.appendChild(meta);
      const check = document.createElement("span");
      check.className = "session-picker-check";
      check.textContent = session.id === activeId ? "\u2713" : "";
      row.append(icon, main, check);
      row.onclick = () => {
        this.dialog.close();
        void this.hooks.openSession(session.id);
      };
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
      this.list.appendChild(row);
    }
  }
}
