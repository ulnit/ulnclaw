import { useEffect, useState } from "react";
import type { GatewayClient, KanbanBoard, KanbanTask } from "../../gateway";
import { Button } from "../../hermes/button";

const COLS = ["todo", "running", "blocked", "done"];

export function KanbanView({ client }: { client: GatewayClient }) {
  const [boards, setBoards] = useState<KanbanBoard[]>([]);
  const [tasks, setTasks] = useState<KanbanTask[]>([]);
  const [board, setBoard] = useState<string | null>(null);

  const load = () => {
    client
      .kanbanBoards()
      .then((b) => {
        setBoards(b);
        const current = b.find((x) => x.current)?.slug ?? b[0]?.slug ?? null;
        setBoard((prev) => prev ?? current);
        return current;
      })
      .then((slug) => {
        if (slug) client.kanbanTasks(slug).then(setTasks).catch(() => undefined);
      })
      .catch(() => undefined);
  };
  useEffect(load, [client]);
  useEffect(() => {
    if (board) client.kanbanTasks(board).then(setTasks).catch(() => undefined);
  }, [board, client]);

  return (
    <div className="h-full overflow-y-auto px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-2 pb-4">
        <h2 className="text-sm font-semibold">Kanban</h2>
        {boards.map((b) => (
          <button
            key={b.slug}
            onClick={() => setBoard(b.slug)}
            className={`rounded-md px-2 py-1 text-xs ${
              board === b.slug ? "bg-(--hover-strong) font-semibold text-(--text)" : "text-(--dim) hover:bg-(--hover)"
            }`}
          >
            {b.name}
          </button>
        ))}
        <Button
          variant="ghost"
          size="xs"
          className="ml-auto"
          onClick={() => {
            const title = window.prompt("Task title");
            if (title && board) {
              void client.kanbanCreateTask(title).then(() => load());
            }
          }}
        >
          + Task
        </Button>
      </div>
      <div className="grid grid-cols-4 gap-3">
        {COLS.map((col) => (
          <div key={col} className="flex min-h-[12rem] flex-col gap-1.5 rounded-lg bg-(--hover) p-2">
            <div className="flex items-center justify-between px-1 pb-1">
              <span className="text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
                {col}
              </span>
              <span className="text-[10px] text-(--dim-faint)">
                {tasks.filter((t) => t.status === col).length}
              </span>
            </div>
            {tasks
              .filter((t) => t.status === col)
              .map((t) => (
                <div key={t.id} className="rounded-md border border-(--border) bg-(--seed-card) px-2.5 py-2">
                  <div className="text-xs font-medium text-(--text)">{t.title}</div>
                  {t.assignee && <div className="pt-0.5 text-[10px] text-(--dim)">{t.assignee}</div>}
                  <div className="flex gap-1 pt-1.5">
                    {col !== "done" && (
                      <Button variant="text" size="micro" onClick={() => void client.kanbanComplete(t.id).then(load)}>
                        complete
                      </Button>
                    )}
                    {col !== "blocked" && (
                      <Button
                        variant="text"
                        size="micro"
                        onClick={() => {
                          const reason = window.prompt("Block reason", "waiting");
                          if (reason) void client.kanbanBlock(t.id, reason).then(load);
                        }}
                      >
                        block
                      </Button>
                    )}
                  </div>
                </div>
              ))}
          </div>
        ))}
      </div>
    </div>
  );
}
