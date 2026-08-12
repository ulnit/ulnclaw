import { useEffect, useState } from "react";
import type { GatewayClient, WebhookListPayload } from "../../gateway";
import { Button } from "../../hermes/button";

export function WebhooksView({ client }: { client: GatewayClient }) {
  const [payload, setPayload] = useState<WebhookListPayload | null>(null);
  const load = () => client.webhooksList().then(setPayload).catch(() => undefined);
  useEffect(() => { load(); }, [client]);
  return (
    <div className="mx-auto w-full max-w-[75rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Webhooks</h2>
        <span className="font-[var(--mono)] text-[11px] text-(--dim)">{payload?.base_url ?? ""}</span>
        <Button
          variant="secondary"
          size="xs"
          className="ml-auto"
          onClick={() => {
            const name = window.prompt("Webhook name");
            const message = window.prompt("Message template", "event fired");
            if (name && message) void client.webhooksCreate({ name, prompt: message }).then(load);
          }}
        >
          + Webhook
        </Button>
      </div>
      <div className="flex flex-col gap-1">
        {(payload?.subscriptions ?? []).map((w) => (
          <div key={w.name} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-2 hover:bg-(--hover)">
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-(--text)">{w.name}</div>
              <div className="truncate text-[11px] text-(--dim)">{w.events?.join(", ") ?? ""}</div>
            </div>
            <div className="flex gap-1">
              <Button variant="ghost" size="xs" onClick={() => void client.webhooksTest(w.name).then(load)}>
                Test
              </Button>
              <Button
                variant="ghost"
                size="xs"
                onClick={() => {
                  if (window.confirm(`Delete webhook ${w.name}?`)) void client.webhooksDelete(w.name).then(load);
                }}
              >
                Delete
              </Button>
            </div>
          </div>
        ))}
        {(payload?.subscriptions.length ?? 0) === 0 && (
          <p className="py-12 text-center text-xs text-(--dim)">No webhook subscriptions</p>
        )}
      </div>
    </div>
  );
}
