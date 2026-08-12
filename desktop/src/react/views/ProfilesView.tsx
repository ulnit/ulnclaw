import { useEffect, useState } from "react";
import type { GatewayClient, ProfileRow } from "../../gateway";
import { Icon } from "../Icon";

export function ProfilesView({ client }: { client: GatewayClient }) {
  const [profiles, setProfiles] = useState<ProfileRow[]>([]);
  useEffect(() => {
    client
      .profilesList()
      .then((p) => {
        const rows = p.profiles;
        setProfiles(rows);
      })
      .catch(() => undefined);
  }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Profiles</h2>
      <div className="flex flex-col gap-1">
        {profiles.map((p, i) => (
          <div key={p.name ?? i} className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)">
            <Icon name="gear" className="size-4 text-(--dim)" />
            <span className="text-[13px] font-medium text-(--text)">
              {p.name}
            </span>
          </div>
        ))}
        {profiles.length === 0 && <p className="py-12 text-center text-xs text-(--dim)">No profiles</p>}
      </div>
    </div>
  );
}
