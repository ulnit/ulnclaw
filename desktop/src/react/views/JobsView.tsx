import { useEffect, useState } from "react";
import type { CronJob, GatewayClient } from "../../gateway";
import { Button } from "../../hermes/button";
import { Icon } from "../Icon";

export function JobsView({ client }: { client: GatewayClient }) {
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const load = () => {
    client.jobsList(true).then(setJobs).catch(() => undefined);
  };
  useEffect(load, [client]);

  const act = async (fn: () => Promise<boolean>) => {
    await fn().catch(() => false);
    load();
  };

  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Jobs</h2>
        <span className="text-xs text-(--dim)">
          {jobs.filter((j) => j.enabled).length} active · {jobs.filter((j) => !j.enabled).length} paused
        </span>
        <Button variant="ghost" size="xs" className="ml-auto" onClick={load}>
          <Icon name="rotate" className="size-3" />
        </Button>
      </div>
      <div className="flex flex-col gap-1">
        {jobs.map((job) => (
          <div
            key={job.id}
            className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)"
          >
            <div className="min-w-0">
              <span className="flex items-center gap-2 text-[13px] font-medium text-(--text)">
                <span className={`size-1.5 rounded-full ${job.enabled ? "bg-(--ok)" : "bg-(--dim-faint)"}`} />
                {job.name}
              </span>
              <span className="mt-0.5 block truncate font-[var(--mono)] text-[11px] text-(--dim)">
                {job.schedule}
              </span>
            </div>
            <div className="flex items-center gap-1">
              <Button
                variant="secondary"
                size="xs"
                onClick={() => void act(() => client.jobSetEnabled(job.id, !job.enabled))}
              >
                <Icon name={job.enabled ? "pause" : "play"} className="size-3" />
                {job.enabled ? "Pause" : "Resume"}
              </Button>
              <Button variant="ghost" size="xs" onClick={() => void act(() => client.jobRunNow(job.id))}>
                <Icon name="zap" className="size-3" />
                Run now
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                title="Delete"
                onClick={() => {
                  if (window.confirm(`Delete job "${job.name}"?`)) {
                    void act(() => client.jobDelete(job.id));
                  }
                }}
              >
                <Icon name="trash" className="size-3 text-(--bad)" />
              </Button>
            </div>
          </div>
        ))}
        {jobs.length === 0 && (
          <p className="py-16 text-center text-xs text-(--dim)">No cron jobs yet</p>
        )}
      </div>
    </div>
  );
}
