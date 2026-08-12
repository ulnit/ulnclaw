import { useEffect, useState } from "react";
import type { DoctorPayload, GatewayClient } from "../../gateway";
import { Button } from "../../hermes/button";
import { Icon } from "../Icon";
import type { IconName } from "../../icons";

const LEVEL_ICON: Record<string, IconName> = { ok: "check", warn: "warn", fail: "x", info: "info" };
const LEVEL_CLASS: Record<string, string> = { ok: "g-ok", warn: "g-warn", fail: "g-bad", info: "g-info" };

export function DoctorView({ client }: { client: GatewayClient }) {
  const [payload, setPayload] = useState<DoctorPayload | null>(null);
  const [logs, setLogs] = useState<string>("");
  const load = () => {
    client.doctor(false).then(setPayload).catch(() => undefined);
    client.logsTail(120).then((p) => setLogs(p.lines.join("\n"))).catch(() => undefined);
  };
  useEffect(() => { load(); }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Doctor</h2>
        <Button variant="ghost" size="xs" className="ml-auto" onClick={load}>
          <Icon name="rotate" className="size-3" />
        </Button>
      </div>
      <div className="flex flex-col gap-5 pb-6">
        {(payload?.report.sections ?? []).map((section) => (
          <section key={section.title}>
            <h3 className="pb-1.5 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
              {section.title}
            </h3>
            {section.checks.map((check, i) => (
              <div key={i} className="flex items-start gap-2.5 rounded-md px-2 py-1.5 hover:bg-(--hover)">
                <span className={`mt-0.5 ${LEVEL_CLASS[check.level]}`}>
                  <Icon name={LEVEL_ICON[check.level] ?? "info"} className="size-3.5" />
                </span>
                <span className="text-[13px] text-(--text)">{check.text}</span>
                {check.detail && <span className="ml-auto truncate text-[11px] text-(--dim)">{check.detail}</span>}
              </div>
            ))}
          </section>
        ))}
        {!payload && <p className="py-12 text-center text-xs text-(--dim)">gateway unreachable</p>}
      </div>
      <h3 className="pb-2 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
        Gateway log tail
      </h3>
      <pre className="max-h-72 overflow-y-auto rounded-lg border border-(--border) bg-transparent p-3 font-[var(--mono)] text-[11px] leading-[1.5] text-(--dim) whitespace-pre-wrap">
        {logs || "no logs"}
      </pre>
    </div>
  );
}
