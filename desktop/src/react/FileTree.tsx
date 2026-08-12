import { useCallback, useEffect, useState } from "react";
import type { FsEntry, GatewayClient } from "../gateway";
import { Button } from "../hermes/button";
import { Icon } from "./Icon";
import { useT } from "./util";

interface Props {
  client: GatewayClient;
  onClose: () => void;
}

export function FileTree({ client, onClose }: Props) {
  const t = useT();
  const [cwd, setCwd] = useState("");
  const [branch, setBranch] = useState("");
  const [entries, setEntries] = useState<FsEntry[]>([]);
  const [preview, setPreview] = useState<{ path: string; text: string; binary: boolean; truncated: boolean } | null>(null);

  const load = useCallback(
    (dir: string) => {
      client
        .fsList(dir)
        .then((p) => {
          setEntries(
            [...p.entries].sort((a, b) =>
              a.isDirectory === b.isDirectory ? a.name.localeCompare(b.name) : a.isDirectory ? -1 : 1,
            ),
          );
        })
        .catch(() => setEntries([]));
    },
    [client],
  );

  useEffect(() => {
    client
      .fsDefaultCwd()
      .then((p) => {
        setCwd(p.cwd);
        setBranch(p.branch);
        load(p.cwd);
      })
      .catch(() => undefined);
  }, [client, load]);

  const open = (entry: FsEntry) => {
    if (entry.isDirectory) {
      setCwd(entry.path);
      load(entry.path);
      return;
    }
    client
      .fsReadText(entry.path)
      .then((p) =>
        setPreview({ path: p.path, text: p.text, binary: p.binary, truncated: p.truncated }),
      )
      .catch(() => setPreview({ path: entry.path, text: "", binary: true, truncated: false }));
  };

  const up = () => {
    const parent = cwd.replace(/[/\\][^/\\]+[/\\]?$/, "") || "/";
    if (parent && parent !== cwd) {
      setCwd(parent);
      load(parent);
    }
  };

  const mkdir = async () => {
    const name = window.prompt(t.chrome.fsMkdirPrompt);
    if (!name) return;
    const sep = cwd.includes("\\") ? "\\" : "/";
    try {
      await client.fsMkdir(`${cwd}${sep}${name}`);
      load(cwd);
    } catch {
      /* fs unavailable */
    }
  };

  return (
    <aside className="flex min-h-0 w-[17rem] flex-col border-l border-(--border) bg-(--panel)">
      <div className="flex items-center gap-1 border-b border-(--border) px-2 py-1.5">
        <Icon name="folder" className="size-3.5" />
        <span className="min-w-0 flex-1 truncate text-xs font-semibold" title={cwd}>
          {cwd.split(/[/\\]/).pop() || "/"}
          {branch && <span className="pl-1.5 font-normal text-(--dim-faint)">{branch}</span>}
        </span>
        <Button variant="ghost" size="icon-xs" title={t.chrome.fsUpTitle} onClick={up}>
          <Icon name="arrowUp" className="size-3" />
        </Button>
        <Button variant="ghost" size="icon-xs" title={t.chrome.fsMkdirTitle} onClick={() => void mkdir()}>
          <Icon name="folderPlus" className="size-3" />
        </Button>
        <Button variant="ghost" size="icon-xs" title={t.kanban.refresh} onClick={() => load(cwd)}>
          <Icon name="rotate" className="size-3" />
        </Button>
        <Button variant="ghost" size="icon-xs" title={t.find.closeTitle} onClick={onClose}>
          <Icon name="x" className="size-3" />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {entries.map((e) => (
          <button
            key={e.path}
            onClick={() => open(e)}
            className="flex w-full items-center gap-1.5 px-2.5 py-1 text-left text-[12.5px] text-(--dim) hover:bg-(--hover) hover:text-(--text)"
          >
            <Icon name={e.isDirectory ? "folder" : "file"} className="size-3 shrink-0" />
            <span className="truncate">{e.name}</span>
          </button>
        ))}
        {entries.length === 0 && (
          <p className="px-3 py-4 text-center text-xs text-(--dim)">{t.chrome.fsEmpty}</p>
        )}
      </div>
      {preview && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setPreview(null)}>
          <div
            className="flex max-h-[80vh] w-[46rem] flex-col rounded-xl border border-(--border) bg-(--seed-card) p-4 shadow-[var(--shadow-composer)]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between pb-2">
              <span className="truncate pr-3 font-mono text-xs font-semibold">{preview.path}</span>
              <div className="flex items-center gap-1">
                <a
                  className="rounded-md px-2 py-1 text-xs text-(--dim) hover:bg-(--hover) hover:text-(--text)"
                  href={client.fsDownloadUrl(preview.path)}
                  target="_blank"
                  rel="noreferrer"
                >
                  {t.chrome.fsDownloadTitle}
                </a>
                <Button variant="ghost" size="xs" onClick={() => setPreview(null)}>
                  {t.chrome.gatewayLogClose}
                </Button>
              </div>
            </div>
            {preview.binary ? (
              <p className="py-8 text-center text-xs text-(--dim)">{t.chrome.fsPreviewBinary}</p>
            ) : (
              <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap rounded-md bg-(--hover) p-2 font-mono text-[11px] leading-4">
                {preview.text}
              </pre>
            )}
            {preview.truncated && (
              <p className="pt-1.5 text-[10px] text-(--dim-faint)">{t.chrome.fsPreviewTruncated}</p>
            )}
          </div>
        </div>
      )}
    </aside>
  );
}
