import { useEffect, useRef, useState } from "react";
import type { GatewayClient, ModelOptionsPayload } from "../gateway";
import { Icon } from "./Icon";
import { useT } from "./util";

interface Props {
  client: GatewayClient;
  onChanged: (model: string) => void;
}

export function ModelPicker({ client, onChanged }: Props) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [payload, setPayload] = useState<ModelOptionsPayload | null>(null);
  const [current, setCurrent] = useState("");
  const [busy, setBusy] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    client
      .modelOptions()
      .then((p) => {
        setPayload(p);
        setCurrent(p.model);
      })
      .catch(() => setPayload(null));
  }, [client, open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const pick = async (provider: string, model: string) => {
    setBusy(true);
    try {
      await client.modelSet(provider, model);
      setCurrent(model);
      onChanged(model);
      setOpen(false);
    } catch {
      /* gateway unreachable */
    }
    setBusy(false);
  };

  const providers = payload?.providers ?? [];
  const authed = providers.filter((p) => p.authenticated);
  const rest = providers.filter((p) => !p.authenticated);

  return (
    <div ref={rootRef} className="relative">
      <button
        className="inline-flex max-w-[16rem] items-center gap-1.5 rounded-md px-2 py-0.5 text-xs text-(--dim) hover:bg-(--hover) hover:text-(--text)"
        title={t.chrome.scModelPicker}
        disabled={busy}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon name="cpu" className="size-3 shrink-0" />
        <span className="truncate">{current || t.chrome.modelsTab}</span>
        <Icon name="chevronDown" className="size-2.5 shrink-0" />
      </button>
      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 max-h-[24rem] w-[22rem] overflow-y-auto rounded-lg border border-(--border) bg-(--seed-card) p-1.5 shadow-[var(--shadow-composer)]">
          {payload === null && (
            <p className="px-2 py-3 text-center text-xs text-(--dim)">{t.boot.unreachable}</p>
          )}
          {payload && providers.length === 0 && (
            <p className="px-2 py-3 text-center text-xs text-(--dim)">no providers</p>
          )}
          {[...authed, ...rest].map((p) => (
            <div key={p.slug} className="pb-1.5">
              <div className="flex items-center gap-1.5 px-2 pt-1.5 pb-0.5">
                <span className="text-[11px] font-semibold text-(--text)">{p.name ?? p.slug}</span>
                {p.current && (
                  <span className="rounded-full bg-(--hover-strong) px-1.5 py-px text-[10px] font-medium text-(--dim)">
                    current
                  </span>
                )}
                {!p.authenticated && (
                  <span className="text-[10px] text-(--dim-faint)">no key</span>
                )}
              </div>
              {p.models.slice(0, 8).map((m) => (
                <button
                  key={m}
                  disabled={!p.authenticated || busy}
                  onClick={() => void pick(p.slug, m)}
                  className={`flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs ${
                    m === current
                      ? "bg-(--hover-strong) font-semibold text-(--text)"
                      : p.authenticated
                        ? "text-(--dim) hover:bg-(--hover) hover:text-(--text)"
                        : "cursor-not-allowed text-(--dim-faint)"
                  }`}
                >
                  <span className="truncate">{m}</span>
                  {m === current && <Icon name="check" className="ml-auto size-3 shrink-0" />}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
