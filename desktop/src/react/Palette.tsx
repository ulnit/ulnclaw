import { useEffect, useState } from "react";
import { Command } from "cmdk";
import type { ShellView } from "./Sidebar";
import { switchShell } from "./util";

export interface PaletteAction {
  id: string;
  label: string;
  hint?: string;
  run: () => void;
}

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onView: (view: ShellView) => void;
  onNewSession: () => void;
}

export function Palette({ open, onOpenChange, onView, onNewSession }: Props) {
  const [value, setValue] = useState("");
  useEffect(() => {
    if (open) setValue("");
  }, [open]);

  if (!open) return null;

  const actions: PaletteAction[] = [
    { id: "new", label: "New session", hint: "chat", run: onNewSession },
    ...(["chat", "sessions", "jobs", "usage", "models", "settings"] as ShellView[]).map(
      (v) => ({ id: `view-${v}`, label: `Go to ${v[0].toUpperCase()}${v.slice(1)}`, run: () => onView(v) }),
    ),
    { id: "classic", label: "Switch to classic shell", run: () => switchShell("vanilla") },
  ];

  return (
    <div
      className="fixed inset-0 z-50 bg-[rgb(8_10_16/0.32)] backdrop-blur-[3px]"
      onClick={() => onOpenChange(false)}
    >
      <Command
        label="Command palette"
        className="fixed left-1/2 top-[18vh] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 rounded-xl border border-[var(--stroke-nous)] bg-(--panel-2) p-1.5 shadow-[var(--shadow-nous)]"
        onClick={(e) => e.stopPropagation()}
      >
        <Command.Input
          autoFocus
          placeholder="Type a command…"
          className="w-full border-b border-(--border) bg-transparent px-2.5 py-2 text-[13px] outline-none placeholder:text-(--dim-faint)"
        />
        <Command.List className="max-h-[18rem] overflow-y-auto p-1">
          <Command.Empty className="px-2.5 py-6 text-center text-xs text-(--dim)">
            No matching command
          </Command.Empty>
          {actions.map((a) => (
            <Command.Item
              key={a.id}
              value={a.label}
              onSelect={() => {
                a.run();
                onOpenChange(false);
              }}
              className="flex cursor-pointer items-center justify-between rounded-md px-2.5 py-1.5 text-[13px] text-(--dim-strong,--dim) aria-selected:bg-(--hover-strong) aria-selected:text-(--text)"
            >
              {a.label}
              {a.hint && <span className="text-[10px] text-(--dim-faint)">{a.hint}</span>}
            </Command.Item>
          ))}
        </Command.List>
        <div className="border-t border-(--border) px-2.5 py-1.5 text-[10px] text-(--dim-faint)">
          Esc closes · Enter runs
        </div>
      </Command>
    </div>
  );
}
