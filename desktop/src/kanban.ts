// Kanban board widget — renders the shared ulnclaw coordination board
// (same kanban.db the CLI `ulnclaw kanban` and the agent kanban_* tools
// use) as a four-column card wall with quick actions.

import type { GatewayClient, KanbanBoard, KanbanTask } from "./gateway";

// Engine statuses (hermes swarm): todo → ready → running → done, plus
// scheduled / blocked / archived. The wall shows four columns; the "To do"
// column gathers todo+ready+scheduled, "Doing" shows running tasks.
const COLUMNS: { key: string; label: string; statuses: string[] }[] = [
  { key: "todo", label: "To do", statuses: ["todo", "ready", "scheduled"] },
  { key: "doing", label: "Doing", statuses: ["running"] },
  { key: "blocked", label: "Blocked", statuses: ["blocked"] },
  { key: "done", label: "Done", statuses: ["done"] },
];

export class KanbanWidget {
  private boards: KanbanBoard[] = [];
  private tasks: KanbanTask[] = [];
  private board = "" as string;
  private timer: number | null = null;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  /** Build the static skeleton once; `refresh` fills it with data. */
  mount(): void {
    this.root.innerHTML = `
      <header id="kanban-header">
        <select id="kanban-board" title="Switch board"></select>
        <span id="kanban-counts" class="kanban-counts"></span>
        <span class="spacer"></span>
        <button id="kanban-refresh" class="ghost" title="Refresh">↻</button>
      </header>
      <div id="kanban-columns"></div>
      <dialog id="kanban-detail">
        <form method="dialog">
          <h2 id="kanban-detail-title"></h2>
          <pre id="kanban-detail-body" class="kanban-detail-body"></pre>
          <div id="kanban-detail-comments" class="kanban-comments"></div>
          <div class="kanban-detail-compose">
            <input id="kanban-comment-input" type="text" placeholder="Add a comment…" />
            <button id="kanban-comment-send" value="comment">Comment</button>
          </div>
          <menu>
            <button id="kanban-detail-unblock" value="unblock">Unblock</button>
            <button id="kanban-detail-block" value="block">Block…</button>
            <button id="kanban-detail-complete" value="complete">Complete</button>
            <button value="cancel">Close</button>
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

    const dialog = this.root.querySelector("#kanban-detail") as HTMLDialogElement;
    dialog.addEventListener("close", () => {
      const id = dialog.dataset.taskId || "";
      if (!id) return;
      const client = this.client();
      if (!client) return;
      if (dialog.returnValue === "complete") {
        void client.kanbanComplete(id).then(() => void this.refresh());
      } else if (dialog.returnValue === "block") {
        const reason = window.prompt("Why is it blocked?");
        if (reason && reason.trim()) {
          void client.kanbanBlock(id, reason.trim()).then(() => void this.refresh());
        }
      } else if (dialog.returnValue === "unblock") {
        void client.kanbanUnblock(id).then(() => void this.refresh());
      } else if (dialog.returnValue === "comment") {
        const input = this.root.querySelector("#kanban-comment-input") as HTMLInputElement;
        const body = input.value.trim();
        if (body) {
          void client.kanbanComment(id, body).then(() => void this.openDetail(id));
        }
      }
    });
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
    counts.textContent = `${open} open · ${this.tasks.length} total`;
  }

  private renderColumns(): void {
    const wrap = this.root.querySelector("#kanban-columns")!;
    wrap.innerHTML = "";
    for (const column of COLUMNS) {
      const tasks = this.tasks
        .filter((t) => column.statuses.includes(t.status))
        .sort((a, b) => b.priority - a.priority || b.created_at - a.created_at);
      const col = document.createElement("div");
      col.className = "kanban-column";
      col.innerHTML = `<div class="kanban-column-header">
          <span>${column.label}</span><span class="kanban-badge">${tasks.length}</span>
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
      done.textContent = "✓ done";
      done.onclick = act(() => client!.kanbanComplete(task.id));
      actions.appendChild(done);
      const block = document.createElement("button");
      block.textContent = "⛔ block";
      block.onclick = act(() => {
        const reason = window.prompt("Why is it blocked?");
        return reason && reason.trim()
          ? client!.kanbanBlock(task.id, reason.trim())
          : Promise.resolve();
      });
      actions.appendChild(block);
    } else if (task.status === "blocked") {
      const unblock = document.createElement("button");
      unblock.textContent = "↩ unblock";
      unblock.onclick = act(() => client!.kanbanUnblock(task.id));
      actions.appendChild(unblock);
    }
    card.appendChild(actions);

    card.onclick = () => void this.openDetail(task.id);
    return card;
  }

  private renderQuickAdd(): HTMLElement {
    const box = document.createElement("div");
    box.className = "kanban-quickadd";
    const input = document.createElement("input");
    input.type = "text";
    input.placeholder = "+ Add a task…";
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
    const parts = [detail.task.body || "(no description)"];
    if (detail.task.result) parts.push(`\nResult: ${detail.task.result}`);
    bodyEl.textContent = parts.join("\n");

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
      comments.innerHTML = `<div class="kanban-comment empty">No comments yet.</div>`;
    }
    (this.root.querySelector("#kanban-comment-input") as HTMLInputElement).value = "";
    const unblockBtn = this.root.querySelector("#kanban-detail-unblock") as HTMLButtonElement;
    unblockBtn.style.display = detail.task.status === "blocked" ? "" : "none";
    const blockBtn = this.root.querySelector("#kanban-detail-block") as HTMLButtonElement;
    blockBtn.style.display = detail.task.status === "blocked" ? "none" : "";
    const completeBtn = this.root.querySelector("#kanban-detail-complete") as HTMLButtonElement;
    completeBtn.style.display = detail.task.status === "done" ? "none" : "";
    dialog.showModal();
  }
}
