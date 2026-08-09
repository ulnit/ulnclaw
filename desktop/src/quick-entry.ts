// Quick Entry — a global-hotkey mini composer (hermes quick-entry
// parity): deliberately one input plus a session-target picker and
// nothing else. This is a capture surface, not a second chat. The
// text goes through the primary composer's normal submit path, and
// the target picker chooses which session receives it (current, a
// fresh one, or a recent session).

import type { SessionRow } from "./gateway";
import { t } from "./i18n";

export interface QuickEntryHooks {
  connected: () => boolean;
  sessions: () => SessionRow[];
  currentSessionId: () => string | null;
  openSession: (session: SessionRow) => Promise<void>;
  newSession: () => void;
  /** Set the primary composer text and submit it. */
  sendText: (text: string) => void;
}

const MAX_RECENT_TARGETS = 8;

export class QuickEntry {
  private overlay!: HTMLDivElement;
  private input!: HTMLInputElement;
  private target!: HTMLSelectElement;
  private hint!: HTMLDivElement;

  constructor(private hooks: QuickEntryHooks) {}

  mount(): void {
    this.overlay = document.createElement("div");
    this.overlay.id = "quick-entry";
    this.overlay.hidden = true;
    this.overlay.innerHTML = `
      <div class="quick-entry-box">
        <span class="quick-entry-caret" aria-hidden>›</span>
        <input id="quick-entry-input" type="text" autocomplete="off" spellcheck="false" />
        <select id="quick-entry-target"></select>
      </div>
      <div id="quick-entry-hint" class="quick-entry-hint"></div>`;
    document.body.appendChild(this.overlay);
    this.input = this.overlay.querySelector("#quick-entry-input") as HTMLInputElement;
    this.target = this.overlay.querySelector("#quick-entry-target") as HTMLSelectElement;
    this.hint = this.overlay.querySelector("#quick-entry-hint") as HTMLDivElement;
    this.input.placeholder = t.quickEntry.placeholder;

    this.input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        void this.submit();
      } else if (event.key === "Escape") {
        event.preventDefault();
        this.hide();
      }
    });
    this.input.addEventListener("blur", (event) => {
      // Moving focus to the target picker is not leaving the overlay.
      if (event.relatedTarget && this.overlay.contains(event.relatedTarget as Node)) return;
      this.hide();
    });
    this.target.addEventListener("blur", (event) => {
      if (event.relatedTarget && this.overlay.contains(event.relatedTarget as Node)) return;
      this.hide();
    });

    window.addEventListener("keydown", (event) => {
      if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.code === "KeyQ") {
        event.preventDefault();
        this.toggle();
      }
    });
  }

  toggle(): void {
    if (this.overlay.hidden) this.show();
    else this.hide();
  }

  show(): void {
    const connected = this.hooks.connected();
    this.input.disabled = !connected;
    this.input.placeholder = connected
      ? t.quickEntry.placeholder
      : t.quickEntry.disconnected;
    this.hint.textContent = connected ? t.quickEntry.hint : t.quickEntry.reconnectHint;
    this.renderTargets();
    this.overlay.hidden = false;
    this.input.value = "";
    requestAnimationFrame(() => this.input.focus());
  }

  hide(): void {
    if (this.overlay.hidden) return;
    this.overlay.hidden = true;
    this.input.value = "";
  }

  private renderTargets(): void {
    const current = this.hooks.currentSessionId();
    this.target.innerHTML = "";
    const currentOption = document.createElement("option");
    currentOption.value = "current";
    currentOption.textContent = t.quickEntry.targetCurrent;
    currentOption.disabled = !current;
    this.target.appendChild(currentOption);
    const newOption = document.createElement("option");
    newOption.value = "new";
    newOption.textContent = t.quickEntry.targetNew;
    this.target.appendChild(newOption);
    const recents = this.hooks
      .sessions()
      .filter((session) => session.id !== current)
      .slice(0, MAX_RECENT_TARGETS);
    for (const session of recents) {
      const option = document.createElement("option");
      option.value = `id:${session.id}`;
      option.textContent = session.title || session.id.slice(0, 8);
      this.target.appendChild(option);
    }
    this.target.value = current ? "current" : "new";
  }

  private async submit(): Promise<void> {
    const text = this.input.value.trim();
    if (!text || !this.hooks.connected()) return;
    const targetValue = this.target.value;
    this.hide();
    if (targetValue === "current") {
      this.hooks.sendText(text);
      return;
    }
    if (targetValue === "new") {
      this.hooks.newSession();
      this.hooks.sendText(text);
      return;
    }
    const id = targetValue.replace(/^id:/, "");
    const session = this.hooks.sessions().find((row) => row.id === id);
    if (!session) {
      this.hooks.sendText(text);
      return;
    }
    await this.hooks.openSession(session);
    this.hooks.sendText(text);
  }
}
