import { useEffect, useState } from "react";
import type { GatewayClient, PairingPlatform } from "../../gateway";
import { Button } from "../../hermes/button";

export function PairingView({ client }: { client: GatewayClient }) {
  const [platforms, setPlatforms] = useState<PairingPlatform[]>([]);
  const load = () => client.pairingStatus().then(setPlatforms).catch(() => undefined);
  useEffect(() => { load(); }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Pairing</h2>
      <div className="flex flex-col gap-6">
        {platforms.map((plat) => (
          <section key={plat.platform}>
            <h3 className="pb-2 text-[13px] font-semibold">{plat.platform}</h3>
            {plat.pending.length > 0 && (
              <div className="pb-3">
                <div className="pb-1 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--warn)">
                  pending
                </div>
                {plat.pending.map((p) => (
                  <div key={p.request_id} className="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-(--hover)">
                    <span className="text-[13px] text-(--text)">{p.user_name || p.user_id}</span>
                    <span className="text-[11px] text-(--dim)">{p.age_minutes}m ago</span>
                    <Button
                      variant="secondary"
                      size="xs"
                      className="ml-auto"
                      onClick={() => {
                        const code = window.prompt("Pairing code");
                        if (code) void client.pairingApprove(plat.platform, code).then(load);
                      }}
                    >
                      Approve
                    </Button>
                  </div>
                ))}
              </div>
            )}
            {plat.approved.map((u) => (
              <div key={u.user_id} className="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-(--hover)">
                <span className="text-[13px] text-(--text)">{u.user_name || u.user_id}</span>
                <Button
                  variant="text"
                  size="micro"
                  className="ml-auto"
                  onClick={() => {
                    if (window.confirm(`Revoke ${u.user_name}?`)) {
                      void client.pairingRevoke(plat.platform, u.user_id).then(load);
                    }
                  }}
                >
                  revoke
                </Button>
              </div>
            ))}
            {plat.pending.length === 0 && plat.approved.length === 0 && (
              <p className="text-xs text-(--dim)">No paired users</p>
            )}
          </section>
        ))}
        {platforms.length === 0 && <p className="py-12 text-center text-xs text-(--dim)">No messaging platforms</p>}
      </div>
    </div>
  );
}
