import { useEffect, useMemo, useState } from "react";
import type { GatewayClient, SessionRow } from "../../gateway";
import { Button } from "../../hermes/button";
import { Icon } from "../Icon";
import { relTime, useT } from "../util";
import { downloadBlob } from "./download";

interface Props {
  client: GatewayClient;
  onOpen: (id: string) => void;
}

export function SessionsView({ client, onOpen }: Props) {
  const [rows, setRows] = useState<SessionRow[]>([]);
  const [filter, setFilter] = useState("");

  const load = () => {
    client.listSessions(true, true).then(setRows).catch(() => undefined);
  };
  useEffect(load, [client]);

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const list = q
      ? rows.filter((s) => `${s.title ?? ""} ${s.last_message ?? ""}`.toLowerCase().includes(q))
      : rows;
    return [...list].sort((a, b) => b.last_activity_at - a.last_activity_at);
  }, [rows, filter]);

  const t = useT();

  const exportMd = async (id: string) => {
    try {
      const { blob, filename } = await client.exportSession(id, "md");
      downloadBlob(blob, filename);
    } catch {
      /* gateway unreachable */
    }
  };

  const rename = async (id: string, current: string) => {
    const title = window.prompt(t.sessionsView.renamePrompt, current);
    if (title === null || title.trim() === "") return;
    try {
      await client.renameSession(id, title.trim());
      load();
    } catch {
      window.alert(t.session.renameFailed);
    }
  };

  const retitle = async (id: string) => {
    try {
      await client.retitleSession(id, true);
      load();
    } catch {
      /* gateway unreachable */
    }
  };

  const toggleArchive = async (id: string, archived: boolean) => {
    try {
      await client.setSessionEndReason(id, archived ? null : "complete");
      load();
    } catch {
      /* gateway unreachable */
    }
  };

  const remove = async (id: string) => {
    if (!window.confirm(t.session.deleteConfirm)) return;
    try {
      await client.deleteSession(id);
      load();
    } catch {
      window.alert(t.session.deleteFailed);
    }
  };

  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Sessions</h2>
        <span className="text-xs text-(--dim)">{visible.length}</span>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter…"
          className="ml-auto w-48 border-b border-(--border) bg-transparent px-0.5 py-1 text-[13px] outline-none placeholder:text-(--dim-faint) focus:border-(--accent)"
        />
        <Button variant="ghost" size="xs" onClick={load}>
          <Icon name="rotate" className="size-3" />
        </Button>
      </div>
      <div className="flex flex-col">
        {visible.map((s) => (
          <div
            key={s.id}
            className="group grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)"
          >
            <button className="min-w-0 text-left" onClick={() => onOpen(s.id)}>
              <span className="block truncate text-[13px] font-medium text-(--text)">
                {s.title || s.last_message || s.id.slice(0, 8)}
              </span>
              <span className="mt-0.5 flex items-center gap-2 text-[11px] text-(--dim)">
                {s.source !== "gateway" && (
                  <span className="rounded-full bg-(--hover) px-1.5 py-px">{s.source}</span>
                )}
                {s.model && <span className="truncate">{s.model}</span>}
                <span>{relTime(s.last_activity_at)}</span>
              </span>
            </button>
            <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100">
              <Button variant="ghost" size="icon-xs" title={t.palette.resumeSession} onClick={() => onOpen(s.id)}>
                <Icon name="fork" className="size-3" />
              </Button>
              <Button variant="ghost" size="icon-xs" title={t.palette.exportMd} onClick={() => void exportMd(s.id)}>
                <Icon name="download" className="size-3" />
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                title={t.palette.retitleCurrent}
                onClick={() => void retitle(s.id)}
              >
                <Icon name="sparkle" className="size-3" />
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                title={t.palette.renameSession}
                onClick={() => void rename(s.id, s.title ?? "")}
              >
                <Icon name="pencil" className="size-3" />
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                title={s.archived ? t.palette.unarchiveSession : t.palette.archiveSession}
                onClick={() => void toggleArchive(s.id, Boolean(s.archived))}
              >
                <Icon name={s.archived ? "unarchive" : "archive"} className="size-3" />
              </Button>
              <Button variant="ghost" size="icon-xs" title={t.palette.deleteSession} onClick={() => void remove(s.id)}>
                <Icon name="trash" className="size-3" />
              </Button>
            </div>
          </div>
        ))}
        {visible.length === 0 && (
          <p className="py-16 text-center text-xs text-(--dim)">No sessions</p>
        )}
      </div>
    </div>
  );
}
