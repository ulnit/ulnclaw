// Session picker (P254) — dependency-free port of hermes apps/desktop
// `components/session-picker.tsx`: the desktop equivalent of the TUI's
// `/resume` sessions overlay. A focused, type-to-filter list of recent
// sessions that resumes the picked one. In hermes the composer dock and
// the /resume + /sessions + /switch slash commands open it so the
// command feels first-class instead of falling through to the headless
// slash worker; the ulnclaw composer intercepts those same commands.

import type { SessionRow } from "./gateway";
import { fmt, t } from "./i18n";

export interface SessionPickerHooks {
  sessions(): SessionRow[];
  currentSessionId(): string | null;
  openSession(id: string): void | Promise<void>;
}

function normalize(query: string): string {
  return query.trim().toLowerCase();
}

export class SessionPickerDialog {
  private dialog: HTMLDialogElement;
  private search: HTMLInputElement;
  private list: HTMLDivElement;

  constructor(private hooks: SessionPickerHooks) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "session-picker-dialog";
    const title = document.createElement("h2");
    title.className = "session-picker-title";
    title.textContent = t.sessionPicker.title;
    this.search = document.createElement("input");
    this.search.type = "text";
    this.search.placeholder = t.sessionPicker.searchPlaceholder;
    this.search.addEventListener("input", () => this.renderList());
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
    this.renderList();
    this.dialog.showModal();
    this.search.focus();
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
    if (!rows.length) {
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
      if (session.project) bits.push(session.project);
      const meta = document.createElement("span");
      meta.className = "session-picker-meta";
      meta.textContent = bits.join(" · ");
      main.appendChild(meta);
      const check = document.createElement("span");
      check.className = "session-picker-check";
      check.textContent = session.id === activeId ? "✓" : "";
      row.append(icon, main, check);
      row.onclick = () => {
        this.dialog.close();
        void this.hooks.openSession(session.id);
      };
      this.list.appendChild(row);
    }
  }
}
