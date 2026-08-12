import { useEffect, useState } from "react";
import type { GatewayClient, Project } from "../../gateway";
import { Icon } from "../Icon";

export function ProjectsView({ client }: { client: GatewayClient }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  useEffect(() => {
    client
      .projectsList()
      .then((p) => {
        setProjects(p.projects);
        setActiveId(p.active_id);
      })
      .catch(() => undefined);
  }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Projects</h2>
      <div className="flex flex-col gap-1">
        {projects.map((p) => (
          <div key={p.id} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)">
            <span
              className="inline-flex size-7 items-center justify-center rounded-md text-sm"
              style={{ background: p.color ? `${p.color}22` : undefined }}
            >
              {p.icon || "📁"}
            </span>
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-[13px] font-medium text-(--text)">
                {p.name}
                {p.id === activeId && (
                  <span className="rounded-full bg-(--hover-strong) px-1.5 py-px text-[10px] text-(--dim)">active</span>
                )}
              </div>
              <div className="truncate text-[11px] text-(--dim)">{p.primary_path ?? p.description ?? ""}</div>
            </div>
            <Icon name="folder" className="size-3.5 text-(--dim-faint)" />
          </div>
        ))}
        {projects.length === 0 && <p className="py-12 text-center text-xs text-(--dim)">No projects yet</p>}
      </div>
    </div>
  );
}
