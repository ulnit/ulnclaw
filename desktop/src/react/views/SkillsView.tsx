import { useEffect, useState } from "react";
import type { GatewayClient, SkillRow } from "../../gateway";

export function SkillsView({ client }: { client: GatewayClient }) {
  const [skills, setSkills] = useState<SkillRow[]>([]);
  const [filter, setFilter] = useState("");
  useEffect(() => {
    client.listSkills().then(setSkills).catch(() => undefined);
  }, [client]);
  const q = filter.trim().toLowerCase();
  const visible = q ? skills.filter((s) => `${s.name} ${s.description}`.toLowerCase().includes(q)) : skills;
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Skills</h2>
        <span className="text-xs text-(--dim)">{visible.length}</span>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter…"
          className="ml-auto w-48 border-b border-(--border) bg-transparent px-0.5 py-1 text-[13px] outline-none placeholder:text-(--dim-faint) focus:border-(--accent)"
        />
      </div>
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        {visible.map((s) => (
          <div key={s.name} className="rounded-md bg-(--hover) px-3 py-2.5">
            <div className="flex items-center gap-2">
              <span className="text-[13px] font-semibold">{s.name}</span>
              {s.category && (
                <span className="rounded-full bg-(--hover-strong) px-1.5 py-px text-[10px] text-(--dim)">
                  {s.category}
                </span>
              )}
            </div>
            <p className="pt-1 text-xs leading-4 text-(--dim)">{s.description}</p>
          </div>
        ))}
        {visible.length === 0 && <p className="col-span-2 py-12 text-center text-xs text-(--dim)">No skills</p>}
      </div>
    </div>
  );
}
