import { useEffect, useState } from "react";
import type { GatewayClient, PluginsPayload } from "../../gateway";
import { Icon } from "../Icon";

export function PluginsView({ client }: { client: GatewayClient }) {
  const [payload, setPayload] = useState<PluginsPayload | null>(null);
  useEffect(() => {
    client.pluginsInventory().then(setPayload).catch(() => undefined);
  }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Plugins</h2>
      <div className="flex flex-col gap-1">
        {(payload?.plugins ?? []).map((p) => (
          <div key={p.name} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)">
            <Icon name="puzzle" className="size-4 text-(--dim)" />
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-(--text)">{p.name}</div>
              <div className="truncate text-[11px] text-(--dim)">{p.description ?? ""}</div>
            </div>
            <span className={`text-[11px] ${payload?.disabled.includes(p.name) ? "text-(--dim-faint)" : "text-(--ok)"}`}>
              {payload?.disabled.includes(p.name) ? "disabled" : "enabled"}
            </span>
          </div>
        ))}
        {(payload?.plugins.length ?? 0) === 0 && (
          <p className="py-12 text-center text-xs text-(--dim)">No plugins installed</p>
        )}
      </div>
    </div>
  );
}
