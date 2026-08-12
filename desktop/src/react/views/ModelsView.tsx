import { useEffect, useState } from "react";
import type { GatewayClient, ModelOptionsPayload } from "../../gateway";

export function ModelsView({ client }: { client: GatewayClient }) {
  const [payload, setPayload] = useState<ModelOptionsPayload | null>(null);
  useEffect(() => {
    client.modelOptions().then(setPayload).catch(() => undefined);
  }, [client]);

  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Models</h2>
        {payload?.provider && (
          <span className="text-xs text-(--dim)">
            {payload.provider}/{payload.model}
          </span>
        )}
      </div>
      <div className="flex flex-col gap-6">
        {(payload?.providers ?? []).map((p) => (
          <section key={p.slug}>
            <div className="flex items-center gap-2 pb-2">
              <h3 className="text-[13px] font-semibold">{p.name || p.slug}</h3>
              {p.current && (
                <span className="rounded-full bg-(--accent-soft,--hover) px-2 py-px text-[10px] font-semibold text-(--accent)">
                  current
                </span>
              )}
              {p.authenticated != null && (
                <span className={`text-[10px] ${p.authenticated ? "text-(--ok)" : "text-(--warn)"}`}>
                  {p.authenticated ? "authenticated" : "not authenticated"}
                </span>
              )}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {p.models.map((m) => (
                <span
                  key={m}
                  className={`rounded-md px-2 py-1 text-xs ${
                    p.current && payload?.model === m
                      ? "bg-(--hover-strong) font-semibold text-(--text)"
                      : "bg-(--hover) text-(--dim-strong,--dim)"
                  }`}
                >
                  {m}
                </span>
              ))}
            </div>
          </section>
        ))}
        {(payload?.providers.length ?? 0) === 0 && (
          <p className="py-16 text-center text-xs text-(--dim)">No providers configured</p>
        )}
      </div>
    </div>
  );
}
