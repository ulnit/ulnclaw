import { useSyncExternalStore } from "react";
import { currentLocale, onLocaleChange, t } from "../i18n";

/** Live translation accessor that re-renders on locale change. */
export function useT() {
  useSyncExternalStore(onLocaleChange, currentLocale);
  return t;
}

export function relTime(epochSeconds: number): string {
  const delta = Math.max(0, Date.now() / 1000 - epochSeconds);
  if (delta < 60) return "now";
  if (delta < 3600) return `${Math.floor(delta / 60)}m`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h`;
  if (delta < 7 * 86400) return `${Math.floor(delta / 86400)}d`;
  return new Date(epochSeconds * 1000).toLocaleDateString();
}

export function switchShell(shell: "vanilla" | "react"): void {
  localStorage.setItem("ulnclaw.shell", shell);
  location.search = "";
  location.reload();
}
