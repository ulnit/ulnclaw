import { useEffect, useState } from "react";
import type { GatewayClient } from "../../gateway";
import { Button } from "../../hermes/button";
import { LOCALE_OPTIONS, currentLocale, setLocale } from "../../i18n";
import { loadBridge, type DesktopBridge } from "../bridge";
import { useT } from "../util";

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
  const t = useT();
  const locale = currentLocale();
  const [bridge, setBridge] = useState<DesktopBridge | null>(null);
  const [autostart, setAutostart] = useState(false);
  const [closeToTray, setCloseToTray] = useState(false);

  useEffect(() => {
    void loadBridge().then((b) => {
      setBridge(b);
      if (!b) return;
      b.autostartEnabled().then(setAutostart).catch(() => undefined);
    });
  }, []);

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
            <span className="text-(--dim-strong,--dim)">Language</span>
            <select
              value={locale}
              onChange={(e) => setLocale(e.target.value as import("../../i18n").Locale)}
              className="rounded-md border border-(--border) bg-(--seed-card) px-2 py-1.5 text-[13px] outline-none focus:border-(--accent)"
            >
              {LOCALE_OPTIONS.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
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



      {bridge && (
        <section className="pb-6">
          <h3 className="pb-2 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
            Desktop
          </h3>
          <div className="flex flex-col gap-2">
            <label className="grid grid-cols-[10rem_1fr] items-center gap-3 text-[13px]">
              <span className="text-(--dim-strong,--dim)">{t.chrome.autostart}</span>
              <input
                type="checkbox"
                checked={autostart}
                onChange={(e) => {
                  setAutostart(e.target.checked);
                  bridge.setAutostart(e.target.checked).catch(() => undefined);
                }}
              />
            </label>
            <label className="grid grid-cols-[10rem_1fr] items-center gap-3 text-[13px]">
              <span className="text-(--dim-strong,--dim)">{t.chrome.hideToTray}</span>
              <input
                type="checkbox"
                checked={closeToTray}
                onChange={(e) => {
                  setCloseToTray(e.target.checked);
                  bridge.setCloseToTray(e.target.checked).catch(() => undefined);
                }}
              />
            </label>
          </div>
        </section>
      )}
    </div>
  );
}
