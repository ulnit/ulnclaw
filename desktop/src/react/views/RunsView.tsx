import { useEffect, useState } from "react";
import type { GatewayClient, RunRow } from "../../gateway";
import { relTime } from "../util";

const tone = (status: string): string =>
  status.includes("fail") || status.includes("error")
    ? "text-(--bad)"
    : status.includes("run") || status.includes("pend")
      ? "text-(--warn)"
      : "text-(--ok)";

export function RunsView({ client }: { client: GatewayClient }) {
  const [runs, setRuns] = useState<RunRow[]>([]);
  useEffect(() => {
    client.listRuns().then(setRuns).catch(() => undefined);
  }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Runs</h2>
      <div className="flex flex-col gap-1">
        {runs.map((r) => (
          <div key={r.run_id} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)">
            <span className={`text-[11px] font-semibold ${tone(r.status)}`}>{r.status}</span>
            <div className="min-w-0">
              <div className="truncate text-[13px] text-(--text)">{r.message}</div>
              {r.error && <div className="truncate text-[11px] text-(--bad)">{r.error}</div>}
            </div>
            <span className="text-[11px] text-(--dim)">{relTime(r.created_at)}</span>
          </div>
        ))}
        {runs.length === 0 && <p className="py-12 text-center text-xs text-(--dim)">No tracked runs</p>}
      </div>
    </div>
  );
}
