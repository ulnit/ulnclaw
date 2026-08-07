// Sessions view — read-only transcript browser over `/api/sessions` +
// `/api/sessions/:id/messages`: filterable session list on the left,
// full transcript render on the right (role headers, tool-call chips
// with expandable arguments), plus Markdown export of the selected
// session. Complements the chat sidebar, which only renders resumable
// sessions for continued conversation.

import type { GatewayClient, MessageRow, SessionRow } from "./gateway";
import { t } from "./i18n";

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

export class SessionsViewWidget {
  private all: SessionRow[] = [];
  private selected: string | null = null;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="sessions-view-header">
        <span id="sessions-view-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="sessions-view-rename" class="ghost" data-i18n-title="sessionsView.renameTitle" hidden>✎</button>
        <button id="sessions-view-delete" class="ghost" data-i18n-title="sessionsView.deleteTitle" hidden>🗑</button>
        <button id="sessions-view-fork" class="ghost" data-i18n-title="sessionsView.forkTitle" hidden>⑂</button>
        <button id="sessions-view-recap" class="ghost" data-i18n="sessionsView.recap" data-i18n-title="sessionsView.recapTitle" hidden></button>
        <button id="sessions-view-export" class="ghost" data-i18n-title="sessionsView.exportTitle" hidden>⭳ MD</button>
        <button id="sessions-view-export-html" class="ghost" data-i18n-title="sessionsView.exportHtmlTitle" hidden>⭳ HTML</button>
        <button id="sessions-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="sessions-view-status" class="config-status" hidden></div>
      <div class="sessions-view-body">
        <div class="sessions-view-listcol">
          <input id="sessions-view-filter" type="search" data-i18n-ph="sessionsView.filterPlaceholder" />
          <input id="sessions-view-search" type="search" data-i18n-ph="sessionsView.searchPlaceholder" />
          <div id="sessions-view-list" class="sessions-view-list"></div>
        </div>
        <div id="sessions-view-transcript" class="sessions-view-transcript">
          <p class="empty" data-i18n="sessionsView.select"></p>
        </div>
      </div>
    `;
    this.root.querySelector("#sessions-view-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-filter")!.addEventListener("input", () => {
      this.renderList();
    });
    let searchDebounce: number | null = null;
    this.root.querySelector("#sessions-view-search")!.addEventListener("input", () => {
      if (searchDebounce !== null) window.clearTimeout(searchDebounce);
      searchDebounce = window.setTimeout(() => {
        this.runSearch().catch(() => undefined);
      }, 350);
    });
    this.root.querySelector("#sessions-view-export")!.addEventListener("click", () => {
      this.exportSelected("md");
    });
    this.root.querySelector("#sessions-view-export-html")!.addEventListener("click", () => {
      this.exportSelected("html");
    });
    this.root.querySelector("#sessions-view-recap")!.addEventListener("click", () => {
      this.toggleRecap().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-fork")!.addEventListener("click", () => {
      this.forkSelected().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-delete")!.addEventListener("click", () => {
      this.deleteSelected().catch(() => undefined);
    });
    this.root.querySelector("#sessions-view-rename")!.addEventListener("click", () => {
      this.renameSelected().catch(() => undefined);
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
      this.all.sort((a, b) => (b.last_activity_at || b.started_at) - (a.last_activity_at || a.started_at));
      (this.root.querySelector("#sessions-view-count") as HTMLElement).textContent =
        t.sessionsView.count.replace("{count}", String(this.all.length));
      this.renderList();
      this.status("");
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
    const filter = (this.root.querySelector("#sessions-view-filter") as HTMLInputElement).value
      .trim()
      .toLowerCase();
    const rows = this.all.filter((session) => {
      if (!filter) return true;
      const haystack = `${session.title || ""} ${session.id} ${session.model || ""} ${session.source || ""}`;
      return haystack.toLowerCase().includes(filter);
    });
    if (rows.length === 0) {
      list.innerHTML = `<p class="empty">${escapeHtml(filter ? t.sessionsView.noMatch : t.sessionsView.empty)}</p>`;
      return;
    }
    list.innerHTML = rows
      .map((session) => {
        const title = session.title || session.id.slice(0, 8);
        const active = session.id === this.selected ? " active" : "";
        return `
          <div class="sessions-view-row${active}" data-id="${escapeHtml(session.id)}">
            <div class="sessions-view-row-title">${escapeHtml(title)}</div>
            <div class="sessions-view-row-meta">
              ${session.model ? `<span class="sessions-view-model">${escapeHtml(session.model)}</span>` : ""}
              <span>${fmtWhen(session.started_at)}</span>
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
  }

  private async loadTranscript(sessionId: string): Promise<void> {
    const client = this.client();
    const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
    const exportBtn = this.root.querySelector("#sessions-view-export") as HTMLButtonElement;
    if (!client) return;
    pane.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.loading)}</p>`;
    try {
      const messages = await client.messages(sessionId);
      if (this.selected !== sessionId) return; // user moved on
      const session = this.all.find((candidate) => candidate.id === sessionId);
      const meta = session ? this.renderTranscriptMeta(session, messages.length) : "";
      pane.innerHTML = meta + this.renderMessages(messages);
      pane.scrollTop = 0;
      exportBtn.hidden = false;
      (this.root.querySelector("#sessions-view-export-html") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-recap") as HTMLButtonElement).hidden = false;
      (this.root.querySelector("#sessions-view-fork") as HTMLButtonElement).hidden = false;
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
      (this.root.querySelector("#sessions-view-recap") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-fork") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-delete") as HTMLButtonElement).hidden = true;
      (this.root.querySelector("#sessions-view-rename") as HTMLButtonElement).hidden = true;
    }
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
            <div class="sessions-view-role">${escapeHtml(this.roleLabel(message.role))}${nameTag}</div>
            ${content}
            ${toolCalls}
          </div>`;
      })
      .join("");
  }

  private async renameSelected(): Promise<void> {
    const client = this.client();
    if (!client || !this.selected) return;
    const sessionId = this.selected;
    const current = this.all.find((session) => session.id === sessionId);
    const title = window.prompt(t.sessionsView.renamePrompt, current?.title || "");
    if (title === null) return;
    try {
      await client.renameSession(sessionId, title.trim());
      this.status(t.sessionsView.renamed, false);
      await this.refresh();
    } catch (error) {
      this.status(
        t.sessionsView.renameFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async deleteSelected(): Promise<void> {
    const client = this.client();
    if (!client || !this.selected) return;
    const sessionId = this.selected;
    const confirmText = t.sessionsView.deleteConfirm.replace("{id}", sessionId.slice(0, 12));
    if (!window.confirm(confirmText)) return;
    try {
      await client.deleteSession(sessionId);
      this.status(t.sessionsView.deleted.replace("{id}", sessionId.slice(0, 12)), false);
      this.selected = null;
      const pane = this.root.querySelector("#sessions-view-transcript") as HTMLElement;
      pane.innerHTML = `<p class="empty">${escapeHtml(t.sessionsView.select)}</p>`;
      for (const id of ["#sessions-view-export", "#sessions-view-export-html", "#sessions-view-recap", "#sessions-view-fork", "#sessions-view-delete", "#sessions-view-rename"]) {
        (this.root.querySelector(id) as HTMLButtonElement).hidden = true;
      }
      await this.refresh();
    } catch (error) {
      this.status(
        t.sessionsView.deleteFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
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

  private exportSelected(format: "md" | "html"): void {
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
}
