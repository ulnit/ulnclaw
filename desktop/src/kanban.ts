// Kanban board widget — renders the shared ulnclaw coordination board
// (same kanban.db the CLI `ulnclaw kanban` and the agent kanban_* tools
// use) as a four-column card wall with quick actions.

import type { GatewayClient, KanbanBoard, KanbanTask } from "./gateway";
import { fmt, t } from "./i18n";

function escapeHtmlKanban(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// Engine statuses (hermes swarm): todo → ready → running → done, plus
// scheduled / blocked / archived. The wall shows four columns; the "To do"
// column gathers todo+ready+scheduled, "Doing" shows running tasks.
const COLUMNS: { key: "todo" | "doing" | "blocked" | "done"; statuses: string[] }[] = [
  { key: "todo", statuses: ["todo", "ready", "scheduled"] },
  { key: "doing", statuses: ["running"] },
  { key: "blocked", statuses: ["blocked"] },
  { key: "done", statuses: ["done", "archived"] },
];

export class KanbanWidget {
  private boards: KanbanBoard[] = [];
  private tasks: KanbanTask[] = [];
  private board = "" as string;
  private timer: number | null = null;
  /** P526: live task-filter text (title / assignee / id). */
  private filterText = "";
  /** P639: whether the detail dialog is in title/body edit mode. */
  private editMode = false;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  /** Build the static skeleton once; `refresh` fills it with data. */
  mount(): void {
    this.root.innerHTML = `
      <header id="kanban-header">
        <select id="kanban-board" title="Switch board" data-i18n-title="kanban.switchBoard"></select>
        <span id="kanban-counts" class="kanban-counts"></span>
        <span id="kanban-dispatch-status" class="config-note"></span>
        <span class="spacer"></span>
        <input id="kanban-filter" type="search" data-i18n-ph="kanban.filterPlaceholder" />
        <button id="kanban-new" class="primary" data-i18n="kanban.newTask">New task</button>
        <button id="kanban-dispatch" class="ghost" data-i18n="kanban.dispatch">Dispatch</button>
        <button id="kanban-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="kanban-columns"></div>
      <dialog id="kanban-detail">
        <form method="dialog">
          <h2 id="kanban-detail-title"></h2>
          <input id="kanban-detail-title-edit" type="text" style="display: none" />
          <div id="kanban-detail-meta" class="kanban-detail-meta"></div>
          <div class="kanban-detail-reasoning">
            <label for="kanban-detail-reasoning" data-i18n="kanban.reasoningLabel">Reasoning effort</label>
            <select id="kanban-detail-reasoning"></select>
          </div>
          <div class="kanban-detail-links">
            <label data-i18n="kanban.linksLabel">Dependencies</label>
            <span id="kanban-detail-parents" class="kanban-links-parents"></span>
            <button id="kanban-detail-link" type="button" data-i18n="kanban.linkAction">Link parent…</button>
          </div>
          <div class="kanban-detail-model">
            <label for="kanban-detail-model" data-i18n="kanban.modelLabel">Model</label>
            <input id="kanban-detail-model" type="text" placeholder="inherit profile" data-i18n-ph="kanban.modelInherit" />
            <input id="kanban-detail-provider" type="text" placeholder="provider (optional)" data-i18n-ph="kanban.providerOptional" />
            <button id="kanban-detail-model-apply" type="button" data-i18n="kanban.applyAction">Apply</button>
          </div>
          <pre id="kanban-detail-body" class="kanban-detail-body"></pre>
          <textarea id="kanban-detail-body-edit" class="kanban-detail-body" style="display: none"></textarea>
          <div id="kanban-detail-attachments" class="kanban-detail-attachments"></div>
          <div id="kanban-detail-events" class="kanban-events"></div>
          <div id="kanban-detail-comments" class="kanban-comments"></div>
          <div class="kanban-detail-compose">
            <input id="kanban-comment-input" type="text" placeholder="Add a comment…" data-i18n-ph="kanban.addComment" />
            <button id="kanban-comment-send" value="comment" data-i18n="kanban.comment">Comment</button>
          </div>
          <menu>
            <button id="kanban-detail-attach" type="button" data-i18n="kanban.attachAction">Attach</button>
            <button id="kanban-detail-edit" type="button" data-i18n="kanban.editAction">Edit</button>
            <button id="kanban-detail-schedule" type="button" data-i18n="kanban.scheduleAction">Schedule</button>
            <button id="kanban-detail-reassign" type="button" data-i18n="kanban.reassignAction">Reassign</button>
            <button id="kanban-detail-archive" type="button" data-i18n="kanban.archiveAction">Archive</button>
            <button id="kanban-detail-delete" type="button" data-i18n="kanban.deleteAction">Delete</button>
            <button id="kanban-detail-claim" value="claim" data-i18n="kanban.claim">Claim</button>
            <button id="kanban-detail-unblock" value="unblock" data-i18n="kanban.unblock">Unblock</button>
            <button id="kanban-detail-block" value="block" data-i18n="kanban.blockEllipsis">Block…</button>
            <button id="kanban-detail-complete" value="complete" data-i18n="kanban.complete">Complete</button>
            <button value="cancel" data-i18n="kanban.close">Close</button>
          </menu>
        </form>
      </dialog>`;

    const select = this.root.querySelector("#kanban-board") as HTMLSelectElement;
    select.onchange = () => {
      const client = this.client();
      if (!client) return;
      void client.kanbanSwitchBoard(select.value).then(() => {
        this.board = select.value;
        void this.refresh();
      });
    };
    (this.root.querySelector("#kanban-refresh") as HTMLButtonElement).onclick = () =>
      void this.refresh();
    // P526: live task filter — re-renders the cached tasks client-side.
    this.root.querySelector("#kanban-filter")!.addEventListener("input", () => {
      this.filterText = (
        (this.root.querySelector("#kanban-filter") as HTMLInputElement).value || ""
      )
        .trim()
        .toLowerCase();
      this.renderColumns();
    });
    (this.root.querySelector("#kanban-dispatch") as HTMLButtonElement).onclick = () =>
      void this.dispatch();
    // P574: quick task creation from the toolbar (palette quick-add parity).
    (this.root.querySelector("#kanban-new") as HTMLButtonElement).onclick = () => {
      const client = this.client();
      if (!client) return;
      const title = window.prompt(t.kanban.newTaskPrompt);
      if (!title || !title.trim()) return;
      void client.kanbanCreateTask(title.trim()).then((task) => {
        if (task) void this.refresh();
      });
    };

    // P645: attach a file path or link to the task.
    (this.root.querySelector("#kanban-detail-attach") as HTMLButtonElement).onclick = () => {
      const dialogEl = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
      const taskId = dialogEl.dataset.taskId || "";
      const client = this.client();
      if (!taskId || !client) return;
      const value = window.prompt(t.kanban.attachPrompt);
      if (!value || !value.trim()) return;
      const trimmed = value.trim();
      const kind = /^https?:\/\//i.test(trimmed) ? "link" : "file";
      void client.kanbanAttach(taskId, kind, trimmed).then((result) => {
        if (!result.ok) {
          window.alert(result.error || "attach failed");
          return;
        }
        void this.openDetail(taskId);
      });
    };

    // P646: schedule + reassign from the detail dialog.
    (this.root.querySelector("#kanban-detail-schedule") as HTMLButtonElement).onclick = () => {
      const dialogEl = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
      const taskId = dialogEl.dataset.taskId || "";
      const client = this.client();
      if (!taskId || !client) return;
      const reason = window.prompt(t.kanban.schedulePrompt) || "";
      void client.kanbanSchedule(taskId, reason.trim()).then((result) => {
        if (!result.ok) {
          window.alert(result.error || "schedule failed");
          return;
        }
        dialogEl.close();
        void this.refresh();
      });
    };
    (this.root.querySelector("#kanban-detail-reassign") as HTMLButtonElement).onclick = () => {
      const dialogEl = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
      const taskId = dialogEl.dataset.taskId || "";
      const client = this.client();
      if (!taskId || !client) return;
      const assignee = window.prompt(t.kanban.reassignPrompt);
      if (assignee === null) return;
      void client.kanbanReassign(taskId, assignee.trim() || "none").then((result) => {
        if (!result.ok) {
          window.alert(result.error || "reassign failed");
          return;
        }
        dialogEl.close();
        void this.refresh();
      });
    };

    // P640: archive / purge from the detail dialog.
    (this.root.querySelector("#kanban-detail-archive") as HTMLButtonElement).onclick = () => {
      const dialogEl = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
      const taskId = dialogEl.dataset.taskId || "";
      const client = this.client();
      if (!taskId || !client) return;
      if (!window.confirm(t.kanban.archiveConfirm)) return;
      void client.kanbanArchive(taskId).then((result) => {
        if (!result.ok) {
          window.alert(result.error || "archive failed");
          return;
        }
        dialogEl.close();
        void this.refresh();
      });
    };
    (this.root.querySelector("#kanban-detail-delete") as HTMLButtonElement).onclick = () => {
      const dialogEl = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
      const taskId = dialogEl.dataset.taskId || "";
      const client = this.client();
      if (!taskId || !client) return;
      if (!window.confirm(t.kanban.deleteConfirm)) return;
      void client.kanbanDelete(taskId).then((result) => {
        if (!result.ok) {
          window.alert(result.error || "delete failed");
          return;
        }
        dialogEl.close();
        void this.refresh();
      });
    };

    // P639: title/body edit toggle in the detail dialog.
    (this.root.querySelector("#kanban-detail-edit") as HTMLButtonElement).onclick = () => {
      const dialogEl = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
      const taskId = dialogEl.dataset.taskId || "";
      if (!taskId) return;
      if (!this.editMode) {
        this.setEditMode(true);
        return;
      }
      const client = this.client();
      if (!client) return;
      const titleInput = this.root.querySelector("#kanban-detail-title-edit") as HTMLInputElement;
      const bodyArea = this.root.querySelector("#kanban-detail-body-edit") as HTMLTextAreaElement;
      const title = titleInput.value.trim();
      const bodyValue = bodyArea.value;
      void client.kanbanEdit(taskId, title, bodyValue).then((result) => {
        if (!result.ok) {
          window.alert(result.error || "edit failed");
          return;
        }
        const titleEl = this.root.querySelector("#kanban-detail-title") as HTMLElement;
        const bodyEl = this.root.querySelector("#kanban-detail-body") as HTMLElement;
        const status = this.tasks.find((task) => task.id === taskId)?.status || "";
        titleEl.textContent = `${title || titleEl.textContent} (${status})`;
        bodyEl.textContent = bodyValue || t.kanban.noDescription;
        this.setEditMode(false);
        void this.refresh();
      });
    };

    const dialog = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
    dialog.addEventListener("close", () => {
      const id = dialog.dataset.taskId || "";
      if (!id) return;
      const client = this.client();
      if (!client) return;
      if (dialog.returnValue === "complete") {
        void client.kanbanComplete(id).then(() => void this.refresh());
      } else if (dialog.returnValue === "block") {
        const reason = window.prompt(t.kanban.whyBlocked);
        if (reason && reason.trim()) {
          void client.kanbanBlock(id, reason.trim()).then(() => void this.refresh());
        }
      } else if (dialog.returnValue === "unblock") {
        void client.kanbanUnblock(id).then(() => void this.refresh());
      } else if (dialog.returnValue === "claim") {
        void client.kanbanClaim(id).then(() => {
          void this.refresh();
          void this.openDetail(id);
        });
      } else if (dialog.returnValue === "comment") {
        const input = this.root.querySelector("#kanban-comment-input") as HTMLInputElement;
        const body = input.value.trim();
        if (body) {
          void client.kanbanComment(id, body).then(() => void this.openDetail(id));
        }
      }
    });
  }

  private async dispatch(): Promise<void> {
    const client = this.client();
    const statusEl = this.root.querySelector("#kanban-dispatch-status") as HTMLElement;
    if (!client) return;
    try {
      const result = await client.kanbanDispatch(false);
      if (result) {
        statusEl.textContent = t.kanban.dispatchResult
          .replace("{spawned}", String(result.spawned))
          .replace("{promoted}", String(result.promoted))
          .replace("{reclaimed}", String(result.reclaimed));
      }
      await this.refresh();
    } catch (error) {
      statusEl.textContent = t.kanban.dispatchFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** Start polling while the tab is visible. */
  start(): void {
    void this.refresh();
    if (this.timer === null) {
      this.timer = window.setInterval(() => void this.refresh(), 5000);
    }
  }

  stop(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
  }

  async refresh(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const [boards, tasks] = await Promise.all([
      client.kanbanBoards(),
      client.kanbanTasks(),
    ]);
    this.boards = boards || [];
    this.tasks = tasks || [];
    this.renderBoards();
    this.renderColumns();
  }

  /** Re-render cached data after a locale switch (P251). */
  rerender(): void {
    if (this.boards.length || this.tasks.length) {
      this.renderBoards();
      this.renderColumns();
    }
  }

  private renderBoards(): void {
    const select = this.root.querySelector("#kanban-board") as HTMLSelectElement;
    select.innerHTML = "";
    for (const board of this.boards) {
      const option = document.createElement("option");
      option.value = board.slug;
      option.textContent = `${board.name} (${board.open_tasks} open)`;
      option.selected = board.current;
      select.appendChild(option);
    }
    const counts = this.root.querySelector("#kanban-counts")!;
    const open = this.tasks.filter((t) => t.status !== "done" && t.status !== "archived").length;
    counts.textContent = fmt(t.kanban.counts, { open, total: this.tasks.length });
  }

  private renderColumns(): void {
    const wrap = this.root.querySelector("#kanban-columns")!;
    wrap.innerHTML = "";
    // P526: narrow the cached tasks across every column by title,
    // assignee, or id before splitting them into the four columns.
    const query = this.filterText;
    const visible = query
      ? this.tasks.filter(
          (task) =>
            task.title.toLowerCase().includes(query) ||
            (task.assignee || "").toLowerCase().includes(query) ||
            task.id.toLowerCase().includes(query),
        )
      : this.tasks;
    for (const column of COLUMNS) {
      const tasks = visible
        .filter((t) => column.statuses.includes(t.status))
        .sort((a, b) => b.priority - a.priority || b.created_at - a.created_at);
      const col = document.createElement("div");
      col.className = "kanban-column";
      col.innerHTML = `<div class="kanban-column-header">
          <span>${t.kanban[column.key]}</span><span class="kanban-badge">${tasks.length}</span>
        </div>`;
      const cards = document.createElement("div");
      cards.className = "kanban-cards";
      for (const task of tasks.slice(0, 100)) {
        cards.appendChild(this.renderCard(task));
      }
      if (column.key === "todo") {
        cards.appendChild(this.renderQuickAdd());
      }
      col.appendChild(cards);
      wrap.appendChild(col);
    }
    if (query && visible.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.kanban.filterNoMatch;
      wrap.appendChild(empty);
    }
  }

  private renderCard(task: KanbanTask): HTMLElement {
    const card = document.createElement("div");
    card.className = `kanban-card status-${task.status}`;
    const title = document.createElement("div");
    title.className = "kanban-card-title";
    title.textContent = task.title;
    card.appendChild(title);

    const meta = document.createElement("div");
    meta.className = "kanban-card-meta";
    const idChip = document.createElement("span");
    idChip.className = "kanban-chip";
    idChip.textContent = task.id.replace(/^t_/, "").slice(0, 6);
    meta.appendChild(idChip);
    if (task.assignee) {
      const assignee = document.createElement("span");
      assignee.className = "kanban-chip assignee";
      assignee.textContent = task.assignee;
      meta.appendChild(assignee);
    }
    if (task.reasoning_effort) {
      const reasoning = document.createElement("span");
      reasoning.className = "kanban-chip";
      reasoning.title = t.kanban.reasoningLabel;
      reasoning.textContent = `\u26A1 ${task.reasoning_effort}`;
      meta.appendChild(reasoning);
    }
    if (task.model) {
      const model = document.createElement("span");
      model.className = "kanban-chip";
      model.title = t.kanban.modelLabel;
      model.textContent = `\uD83E\uDDE0 ${task.model}`;
      meta.appendChild(model);
    }
    if (task.children.length > 0) {
      const sub = document.createElement("span");
      sub.className = "kanban-chip";
      sub.textContent = `⤷ ${task.children.length}`;
      meta.appendChild(sub);
    }
    card.appendChild(meta);

    const actions = document.createElement("div");
    actions.className = "kanban-card-actions";
    const client = this.client();
    const act = (fn: () => Promise<unknown>) => (event: MouseEvent) => {
      event.stopPropagation();
      if (client) void fn().then(() => void this.refresh());
    };
    if (["todo", "ready", "scheduled", "running"].includes(task.status)) {
      const done = document.createElement("button");
      done.textContent = t.kanban.doneAction;
      done.onclick = act(() => client!.kanbanComplete(task.id));
      actions.appendChild(done);
      const block = document.createElement("button");
      block.textContent = t.kanban.blockAction;
      block.onclick = act(() => {
        const reason = window.prompt(t.kanban.whyBlocked);
        return reason && reason.trim()
          ? client!.kanbanBlock(task.id, reason.trim())
          : Promise.resolve();
      });
      actions.appendChild(block);
    } else if (task.status === "blocked") {
      const unblock = document.createElement("button");
      unblock.textContent = t.kanban.unblockAction;
      unblock.onclick = act(() => client!.kanbanUnblock(task.id));
      actions.appendChild(unblock);
    }
    if (task.status !== "archived") {
      const archive = document.createElement("button");
      archive.textContent = "\uD83D\uDCE6";
      archive.title = t.kanban.archiveAction;
      archive.onclick = act(() => {
        if (!window.confirm(t.kanban.archiveConfirm)) return Promise.resolve();
        return client!.kanbanArchive(task.id);
      });
      actions.appendChild(archive);
    } else {
      const purge = document.createElement("button");
      purge.textContent = "\uD83D\uDDD1";
      purge.title = t.kanban.deleteAction;
      purge.onclick = act(() => {
        if (!window.confirm(t.kanban.deleteConfirm)) return Promise.resolve();
        return client!.kanbanDelete(task.id);
      });
      actions.appendChild(purge);
    }
    // P571: duplicate the task (title + body) with a localized suffix.
    const dup = document.createElement("button");
    dup.textContent = "\u29C9";
    dup.title = t.kanban.duplicateAction;
    dup.onclick = act(() =>
      client!.kanbanCreateTask(task.title + t.kanban.duplicateSuffix, task.body || undefined),
    );
    actions.appendChild(dup);
    card.appendChild(actions);

    card.onclick = () => void this.openDetail(task.id);
    return card;
  }

  /** P639: flip the detail dialog between display and edit mode. */
  private setEditMode(editing: boolean): void {
    this.editMode = editing;
    const titleEl = this.root.querySelector("#kanban-detail-title") as HTMLElement;
    const titleInput = this.root.querySelector("#kanban-detail-title-edit") as HTMLInputElement;
    const bodyEl = this.root.querySelector("#kanban-detail-body") as HTMLElement;
    const bodyArea = this.root.querySelector("#kanban-detail-body-edit") as HTMLTextAreaElement;
    const editBtn = this.root.querySelector("#kanban-detail-edit") as HTMLButtonElement;
    titleEl.style.display = editing ? "none" : "";
    titleInput.style.display = editing ? "" : "none";
    bodyEl.style.display = editing ? "none" : "";
    bodyArea.style.display = editing ? "" : "none";
    editBtn.textContent = editing ? t.kanban.saveAction : t.kanban.editAction;
    if (editing) titleInput.focus();
  }

  private renderQuickAdd(): HTMLElement {
    const box = document.createElement("div");
    box.className = "kanban-quickadd";
    const input = document.createElement("input");
    input.type = "text";
    input.placeholder = t.kanban.addTask;
    const submit = () => {
      const title = input.value.trim();
      const client = this.client();
      if (!title || !client) return;
      input.value = "";
      void client.kanbanCreateTask(title).then(() => void this.refresh());
    };
    input.onkeydown = (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        submit();
      }
    };
    box.appendChild(input);
    return box;
  }

  private async openDetail(id: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    const detail = await client.kanbanTask(id);
    if (!detail) return;
    const dialog = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
    dialog.dataset.taskId = id;
    (this.root.querySelector("#kanban-detail-title") as HTMLElement).textContent =
      `${detail.task.title} (${detail.task.status})`;
    const bodyEl = this.root.querySelector("#kanban-detail-body") as HTMLElement;
    const parts = [detail.task.body || t.kanban.noDescription];
    if (detail.task.result) parts.push(`\n${fmt(t.kanban.resultPrefix, { result: detail.task.result })}`);
    bodyEl.textContent = parts.join("\n");

    const metaEl = this.root.querySelector("#kanban-detail-meta") as HTMLElement;
    const task = detail.task;
    const fmtWhen = (ts: number | null): string =>
      ts ? new Date(ts * 1000).toLocaleString() : "—";
    const metaParts = [
      `${t.kanban.metaAssignee}: ${task.assignee || "—"}`,
      `${t.kanban.metaPriority}: P${task.priority}`,
      `${t.kanban.metaCreated}: ${fmtWhen(task.created_at)}`,
    ];
    if (task.created_by) metaParts.push(`${t.kanban.metaCreatedBy}: ${task.created_by}`);
    if (task.started_at) metaParts.push(`${t.kanban.metaStarted}: ${fmtWhen(task.started_at)}`);
    if (task.completed_at) metaParts.push(`${t.kanban.metaCompleted}: ${fmtWhen(task.completed_at)}`);
    if (task.parents.length > 0) metaParts.push(`${t.kanban.metaParents}: ${task.parents.join(", ")}`);
    if (task.children.length > 0) metaParts.push(`${t.kanban.metaChildren}: ${task.children.join(", ")}`);
    metaEl.textContent = metaParts.join(" · ");

    // P636: per-task reasoning-effort pin (hermes reasoning_effort);
    // "" = inherit the worker profile's own agent.reasoning_effort.
    const reasoningSelect = this.root.querySelector(
      "#kanban-detail-reasoning",
    ) as HTMLSelectElement;
    reasoningSelect.innerHTML = "";
    const levels = ["", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
    for (const level of levels) {
      const option = document.createElement("option");
      option.value = level;
      option.textContent = level === "" ? t.kanban.reasoningInherit : level;
      reasoningSelect.appendChild(option);
    }
    reasoningSelect.value = task.reasoning_effort || "";
    reasoningSelect.onchange = () => {
      const c = this.client();
      if (!c) return;
      const value = reasoningSelect.value === "" ? null : reasoningSelect.value;
      void c.kanbanSetReasoning(task.id, value).then(() => void this.refresh());
    };

    // P637: per-task model/provider pin (hermes kanban set-model);
    // empty model clears both. Provider without a model is refused.
    const modelInput = this.root.querySelector("#kanban-detail-model") as HTMLInputElement;
    const providerInput = this.root.querySelector(
      "#kanban-detail-provider",
    ) as HTMLInputElement;
    modelInput.value = task.model || "";
    providerInput.value = task.provider || "";
    (this.root.querySelector("#kanban-detail-model-apply") as HTMLButtonElement).onclick = () => {
      const c = this.client();
      if (!c) return;
      const model = modelInput.value.trim();
      const provider = providerInput.value.trim();
      void c
        .kanbanSetModel(task.id, model === "" ? null : model, provider === "" ? null : provider)
        .then((result) => {
          if (!result.ok) {
            window.alert(result.error || "set-model failed");
            return;
          }
          void this.refresh();
        });
    };

    const attachEl = this.root.querySelector("#kanban-detail-attachments") as HTMLElement;
    attachEl.innerHTML = "";
    {
      const heading = document.createElement("h3");
      heading.className = "config-section";
      heading.textContent = t.kanban.attachmentsTitle;
      attachEl.appendChild(heading);
      for (const attachment of detail.attachments) {
        const row = document.createElement("div");
        row.className = "kanban-attachment";
        const badge = document.createElement("span");
        badge.className = "models-view-badge";
        badge.textContent = attachment.kind;
        const value = document.createElement("code");
        value.textContent = attachment.value;
        const remove = document.createElement("button");
        remove.textContent = "\u2715";
        remove.title = t.kanban.removeAttachmentTitle;
        remove.onclick = () => {
          const c = this.client();
          if (!c) return;
          void c.kanbanRemoveAttachment(attachment.id).then((result) => {
            if (!result.ok) {
              window.alert(result.error || "remove failed");
              return;
            }
            void this.openDetail(task.id);
          });
        };
        row.append(badge, document.createTextNode(" "), value, remove);
        attachEl.appendChild(row);
      }
    }

    // P647: dependency chips — ✕ unlinks a parent; Link parent… adds.
    const parentsEl = this.root.querySelector("#kanban-detail-parents") as HTMLElement;
    parentsEl.innerHTML = "";
    for (const parentId of task.parents) {
      const chip = document.createElement("span");
      chip.className = "kanban-chip";
      chip.textContent = parentId.replace(/^t_/, "").slice(0, 6);
      chip.title = parentId;
      const unlink = document.createElement("button");
      unlink.textContent = "\u2715";
      unlink.title = t.kanban.unlinkTitle;
      unlink.onclick = () => {
        const c = this.client();
        if (!c) return;
        void c.kanbanUnlink(task.id, parentId).then((result) => {
          if (!result.ok) {
            window.alert(result.error || "unlink failed");
            return;
          }
          void this.refresh();
          void this.openDetail(task.id);
        });
      };
      chip.appendChild(unlink);
      parentsEl.appendChild(chip);
    }
    (this.root.querySelector("#kanban-detail-link") as HTMLButtonElement).onclick = () => {
      const c = this.client();
      if (!c) return;
      const parent = window.prompt(t.kanban.linkPrompt);
      if (!parent || !parent.trim()) return;
      void c.kanbanLink(task.id, parent.trim()).then((result) => {
        if (!result.ok) {
          window.alert(result.error || "link failed");
          return;
        }
        void this.refresh();
        void this.openDetail(task.id);
      });
    };

    // P638: task activity trail (hermes event log) — kind + compact
    // payload per event, newest last like the CLI `kanban log`.
    const eventsEl = this.root.querySelector("#kanban-detail-events") as HTMLElement;
    eventsEl.innerHTML = "";
    if (detail.events.length > 0) {
      const heading = document.createElement("h3");
      heading.className = "config-section";
      heading.textContent = t.kanban.eventsTitle;
      eventsEl.appendChild(heading);
      for (const event of detail.events) {
        const row = document.createElement("div");
        row.className = "kanban-event";
        const when = new Date(event.created_at * 1000).toLocaleString();
        const payloadEntries = Object.entries(event.payload || {});
        const payloadText = payloadEntries
          .map(([key, value]) => `${key}=${typeof value === "string" ? value : JSON.stringify(value)}`)
          .join(" ");
        const head = document.createElement("span");
        head.className = "kanban-event-head";
        head.textContent = payloadText ? `${event.kind} — ${payloadText}` : event.kind;
        const stamp = document.createElement("span");
        stamp.className = "kanban-event-when";
        stamp.textContent = when;
        row.appendChild(head);
        row.appendChild(stamp);
        eventsEl.appendChild(row);
      }
    }

    const comments = this.root.querySelector("#kanban-detail-comments")!;
    comments.innerHTML = "";
    for (const comment of detail.comments) {
      const row = document.createElement("div");
      row.className = "kanban-comment";
      const when = new Date(comment.created_at * 1000).toLocaleString();
      row.innerHTML = `<span class="kanban-comment-author"></span> <span class="kanban-comment-when">${when}</span>`;
      (row.querySelector(".kanban-comment-author") as HTMLElement).textContent = comment.author;
      const body = document.createElement("div");
      body.textContent = comment.body;
      row.appendChild(body);
      comments.appendChild(row);
    }
    if (detail.comments.length === 0) {
      comments.innerHTML = "";
      const empty = document.createElement("div");
      empty.className = "kanban-comment empty";
      empty.textContent = t.kanban.noComments;
      comments.appendChild(empty);
    }
    // P639: always reopen in display mode with prefilled edit fields.
    this.setEditMode(false);
    (this.root.querySelector("#kanban-detail-title-edit") as HTMLInputElement).value =
      detail.task.title;
    (this.root.querySelector("#kanban-detail-body-edit") as HTMLTextAreaElement).value =
      detail.task.body || "";
    (this.root.querySelector("#kanban-comment-input") as HTMLInputElement).value = "";
    const unblockBtn = this.root.querySelector("#kanban-detail-unblock") as HTMLButtonElement;
    unblockBtn.style.display = detail.task.status === "blocked" ? "" : "none";
    const blockBtn = this.root.querySelector("#kanban-detail-block") as HTMLButtonElement;
    blockBtn.style.display = detail.task.status === "blocked" ? "none" : "";
    const completeBtn = this.root.querySelector("#kanban-detail-complete") as HTMLButtonElement;
    completeBtn.style.display = detail.task.status === "done" ? "none" : "";
    const claimBtn = this.root.querySelector("#kanban-detail-claim") as HTMLButtonElement;
    claimBtn.style.display = detail.task.status === "done" ? "none" : "";
    const scheduleBtn = this.root.querySelector("#kanban-detail-schedule") as HTMLButtonElement;
    scheduleBtn.style.display = ["todo", "ready", "blocked"].includes(detail.task.status) ? "" : "none";
    const reassignBtn = this.root.querySelector("#kanban-detail-reassign") as HTMLButtonElement;
    reassignBtn.style.display = ["done", "archived"].includes(detail.task.status) ? "none" : "";
    const archiveBtn = this.root.querySelector("#kanban-detail-archive") as HTMLButtonElement;
    archiveBtn.style.display = detail.task.status === "archived" ? "none" : "";
    const deleteBtn = this.root.querySelector("#kanban-detail-delete") as HTMLButtonElement;
    deleteBtn.style.display = detail.task.status === "archived" ? "" : "none";
    if (!dialog.open) dialog.showModal();
  }
}
