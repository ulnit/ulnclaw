import { useEffect, useState } from "react";
import type { GatewayClient } from "../../gateway";
import { Button } from "../../hermes/button";
import { switchShell } from "../util";

interface ThemeRow {
  name: string;
  label: string;
  description?: string;
}

export function SettingsView({ client }: { client: GatewayClient }) {
  const [themes, setThemes] = useState<ThemeRow[]>([]);
  const [active, setActive] = useState("");
  const [fonts, setFonts] = useState<string[]>([]);
  const [font, setFont] = useState("theme");
  const [shell, setShell] = useState(localStorage.getItem("ulnclaw.shell") || "vanilla");

  useEffect(() => {
    client
      .dashboardThemes()
      .then((p) => {
        setThemes(p.themes);
        setActive(p.active);
        document.documentElement.dataset.theme = p.active;
      })
      .catch(() => undefined);
    client
      .dashboardFontInfo()
      .then((p) => {
        setFonts(p.font_choices);
        setFont(p.font);
      })
      .catch(() => undefined);
  }, [client]);

  const pickTheme = (name: string) => {
    setActive(name);
    document.documentElement.dataset.theme = name;
    client.dashboardSetTheme(name).catch(() => undefined);
  };

  const pickFont = (id: string) => {
    setFont(id);
    if (id === "theme") delete document.documentElement.dataset.font;
    else document.documentElement.dataset.font = id;
    client.dashboardSetFont(id).catch(() => undefined);
  };

  const pickShell = (next: string) => {
    setShell(next);
    localStorage.setItem("ulnclaw.shell", next);
  };

  return (
    <div className="mx-auto w-full max-w-[46rem] px-[clamp(1.25rem,4vw,4rem)] py-6">
      <h2 className="pb-4 text-sm font-semibold">Settings</h2>

      <section className="pb-6">
        <h3 className="pb-2 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
          Appearance
        </h3>
        <div className="flex flex-col gap-2">
          <label className="grid grid-cols-[10rem_1fr] items-center gap-3 text-[13px]">
            <span className="text-(--dim-strong,--dim)">Theme</span>
            <select
              value={active}
              onChange={(e) => pickTheme(e.target.value)}
              className="rounded-md border border-(--border) bg-(--seed-card) px-2 py-1.5 text-[13px] outline-none focus:border-(--accent)"
            >
              {themes.map((t) => (
                <option key={t.name} value={t.name}>
                  {t.label}
                </option>
              ))}
            </select>
          </label>
          <label className="grid grid-cols-[10rem_1fr] items-center gap-3 text-[13px]">
            <span className="text-(--dim-strong,--dim)">Font</span>
            <select
              value={font}
              onChange={(e) => pickFont(e.target.value)}
              className="rounded-md border border-(--border) bg-(--seed-card) px-2 py-1.5 text-[13px] outline-none focus:border-(--accent)"
            >
              <option value="theme">theme</option>
              {fonts.map((f) => (
                <option key={f} value={f}>
                  {f}
                </option>
              ))}
            </select>
          </label>
        </div>
      </section>

      <section className="pb-6">
        <h3 className="pb-2 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
          Shell
        </h3>
        <div className="flex items-center gap-2">
          {(
            [
              ["react", "React shell (beta)"],
              ["vanilla", "Classic shell"],
            ] as const
          ).map(([id, label]) => (
            <Button
              key={id}
              variant={shell === id ? "secondary" : "ghost"}
              size="xs"
              onClick={() => pickShell(id)}
            >
              {label}
            </Button>
          ))}
          <span className="pl-2 text-[11px] text-(--dim)">applies on next launch</span>
        </div>
        <p className="pt-3 text-[11px] leading-4 text-(--dim)">
          Provider keys, approvals and gateway knobs still live in the classic
          settings dialog while the migration completes.
        </p>
        <Button variant="outline" size="xs" className="mt-2" onClick={() => switchShell("vanilla")}>
          Open classic settings
        </Button>
      </section>
    </div>
  );
}
