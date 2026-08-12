import { useMemo, useState } from "react";
import type { SessionRow } from "../gateway";
import { Button } from "../hermes/button";
import { relTime, switchShell } from "./util";

export type ShellView = "chat" | "sessions" | "jobs" | "usage" | "models" | "settings";

interface Props {
  sessions: SessionRow[];
  activeId: string | null;
  online: boolean;
  view: ShellView;
  onView: (view: ShellView) => void;
  onSelect: (id: string) => void;
  onRefresh: () => Promise<void>;
}

export function Sidebar({ sessions, activeId, online, view, onView, onSelect, onRefresh }: Props) {
  const [filter, setFilter] = useState("");
  const [busy, setBusy] = useState(false);

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const rows = q
      ? sessions.filter((s) =>
          `${s.title ?? ""} ${s.last_message ?? ""} ${s.id}`.toLowerCase().includes(q),
        )
      : sessions;
    return [...rows].sort((a, b) => b.last_activity_at - a.last_activity_at);
  }, [sessions, filter]);

  const newSession = () => {
    onView("chat");
    onSelect("");
  };

  return (
    <aside className="flex min-h-0 flex-col gap-2 border-r border-[color-mix(in_srgb,var(--seed-fg)_7.5%,transparent)] bg-(--panel) px-2.5 py-3">
      <div className="flex items-center justify-between px-1.5 pb-1">
        <span className="text-[13px] font-semibold tracking-[0.01em]">ulnclaw</span>
        <span
          className={`inline-block size-2.5 rounded-full ${online ? "bg-(--ok)" : "bg-(--bad)"}`}
          title={online ? "gateway up" : "gateway down"}
        />
      </div>

      <div className="flex flex-wrap gap-0.5 px-0.5 pb-2">
        {(
          [
            ["chat", "Chat"],
            ["sessions", "Sessions"],
            ["jobs", "Jobs"],
            ["usage", "Usage"],
            ["models", "Models"],
          ] as [ShellView, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            onClick={() => onView(id)}
            className={`rounded-md px-2 py-1 text-xs ${
              view === id
                ? "bg-(--hover-strong) font-semibold text-(--text)"
                : "font-medium text-(--dim) hover:bg-(--hover) hover:text-(--text)"
            }`}
          >
            {label}
          </button>
        ))}
        <button
          onClick={() => onView("settings")}
          className={`rounded-md px-2 py-1 text-xs ${
            view === "settings"
              ? "bg-(--hover-strong) font-semibold text-(--text)"
              : "font-medium text-(--dim) hover:bg-(--hover) hover:text-(--text)"
          }`}
        >
          Settings
        </button>
      </div>

      <Button variant="default" className="w-full" disabled={busy} onClick={() => void newSession()}>
        New session
      </Button>

      <div className="flex items-center gap-1 px-0.5">
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter sessions…"
          className="min-w-0 flex-1 border-b border-(--border) bg-transparent px-0.5 py-1 text-[13px] outline-none placeholder:text-(--dim-faint) focus:border-(--accent)"
        />
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto">
        {visible.length === 0 && (
          <p className="px-2 py-3 text-center text-xs text-(--dim)">No sessions yet</p>
        )}
        {visible.map((s) => {
          const label = s.title || s.last_message || s.id.slice(0, 8);
          const active = s.id === activeId;
          return (
            <button
              key={s.id}
              onClick={() => onSelect(s.id)}
              className={`grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-2 rounded-md px-2 py-1.5 text-left ${
                active ? "bg-(--hover-strong)" : "hover:bg-(--hover)"
              }`}
            >
              <span className="truncate text-[13px] font-medium text-(--text)">{label}</span>
              <span className="text-[11px] text-(--dim)">{relTime(s.last_activity_at)}</span>
            </button>
          );
        })}
      </div>

      <div className="flex items-center justify-between border-t border-(--border) pt-2">
        <Button variant="text" size="micro" onClick={() => switchShell("vanilla")}>
          Classic shell
        </Button>
        <Button variant="ghost" size="micro" onClick={() => void onRefresh()}>
          Refresh
        </Button>
      </div>
    </aside>
  );
}
