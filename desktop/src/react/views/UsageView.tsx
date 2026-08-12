import { useEffect, useState } from "react";
import type { GatewayClient, ModelUsageRow, UsagePayload } from "../../gateway";

const fmt = (n: number): string => n.toLocaleString();

export function UsageView({ client }: { client: GatewayClient }) {
  const [usage, setUsage] = useState<UsagePayload | null>(null);
  const [models, setModels] = useState<ModelUsageRow[]>([]);

  useEffect(() => {
    client.usage(50).then(setUsage).catch(() => undefined);
    client.analyticsModels(30).then((r) => setModels(r.models)).catch(() => undefined);
  }, [client]);

  const stats: [string, string][] = usage
    ? [
        ["Total tokens", fmt(usage.process.total_tokens)],
        ["Prompt", fmt(usage.process.prompt_tokens)],
        ["Completion", fmt(usage.process.completion_tokens)],
        ["Tool calls", fmt(usage.process.tool_calls)],
        ["Sessions", fmt(usage.store.sessions)],
        ["Runs", fmt(usage.process.runs.completed)],
      ]
    : [];

  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Usage</h2>
      <div className="grid grid-cols-3 gap-2 pb-6 md:grid-cols-6">
        {stats.map(([label, value]) => (
          <div key={label} className="rounded-md bg-(--hover) px-3 py-2.5">
            <div className="text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
              {label}
            </div>
            <div className="pt-0.5 text-sm font-semibold tabular-nums">{value}</div>
          </div>
        ))}
      </div>
      <table className="w-full border-collapse text-[13px]">
        <thead>
          <tr className="text-left text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
            <th className="py-1.5 pr-3 font-semibold">Model</th>
            <th className="py-1.5 pr-3 font-semibold">Sessions</th>
            <th className="py-1.5 pr-3 font-semibold">Messages</th>
            <th className="py-1.5 text-right font-semibold">Tokens</th>
          </tr>
        </thead>
        <tbody>
          {models.map((m) => (
            <tr key={m.model} className="border-t border-(--border)">
              <td className="py-2 pr-3 font-medium text-(--text)">{m.model}</td>
              <td className="py-2 pr-3 text-(--dim) tabular-nums">{m.sessions}</td>
              <td className="py-2 pr-3 text-(--dim) tabular-nums">{m.messages}</td>
              <td className="py-2 text-right tabular-nums">{fmt(m.total_tokens)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {models.length === 0 && <p className="py-10 text-center text-xs text-(--dim)">No usage yet</p>}
    </div>
  );
}
