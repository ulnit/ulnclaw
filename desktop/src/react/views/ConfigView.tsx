import { useEffect, useState } from "react";
import type { GatewayClient } from "../../gateway";
import { Button } from "../../hermes/button";

export function ConfigView({ client }: { client: GatewayClient }) {
  const [toml, setToml] = useState("");
  const [path, setPath] = useState("");
  const [saved, setSaved] = useState("");
  useEffect(() => {
    client
      .configRaw()
      .then((r) => {
        setToml(r.toml);
        setPath(r.path);
      })
      .catch(() => undefined);
  }, [client]);

  const save = async () => {
    try {
      await client.saveConfigRaw(toml);
      setSaved("saved");
      setTimeout(() => setSaved(""), 2000);
    } catch (e) {
      setSaved(`error: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  return (
    <div className="mx-auto flex h-full w-full max-w-[75rem] flex-col px-[clamp(1.25rem,4vw,4rem)] py-6">
      <div className="flex items-center gap-3 pb-4">
        <h2 className="text-sm font-semibold">Config</h2>
        <span className="truncate font-[var(--mono)] text-[11px] text-(--dim)">{path}</span>
        <span className="ml-auto text-[11px] text-(--ok)">{saved}</span>
        <Button variant="default" size="xs" onClick={() => void save()}>
          Save
        </Button>
      </div>
      <textarea
        value={toml}
        onChange={(e) => setToml(e.target.value)}
        spellCheck={false}
        className="min-h-0 flex-1 resize-none rounded-lg border border-(--border) bg-(--seed-card) p-3 font-[var(--mono)] text-[12px] leading-[1.5] outline-none focus:border-(--accent)"
      />
    </div>
  );
}
