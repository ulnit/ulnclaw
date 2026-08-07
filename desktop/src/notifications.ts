// Desktop notification stacks (P249) — scoped port of hermes
// apps/desktop `store/notifications.ts` + `components/notifications.tsx`
// to dependency-free TypeScript.
//
// Two stacks, hermes semantics:
//   * top-center — errors, warnings and anything with an action button;
//     collapsed to the latest toast with a "+N more" expander + clear-all;
//     error/warning are sticky (duration 0).
//   * bottom-right — ambient confirmations (saved/enabled/…); auto-dismiss
//     after 5 s, newest at the bottom, capped.

import { t } from "./i18n";

export type NotificationKind = "error" | "warning" | "info" | "success";

export interface NotificationAction {
  label: string;
  onClick: () => void;
}

export type NotificationPlacement = "default" | "bottom-right";

export interface NotificationInput {
  id?: string;
  kind?: NotificationKind;
  title?: string;
  message: string;
  detail?: string;
  meta?: string;
  action?: NotificationAction;
  durationMs?: number;
  placement?: NotificationPlacement;
}

interface AppNotification extends NotificationInput {
  id: string;
  kind: NotificationKind;
  createdAt: number;
  placement: NotificationPlacement;
}

const KIND_ICON: Record<NotificationKind, string> = {
  error: "⛔",
  warning: "⚠️",
  info: "ℹ️",
  success: "✅",
};

const AMBIENT_CAP = 4;

let counter = 0;
const items: AppNotification[] = [];
const timers = new Map<string, number>();
let expanded = false;

function defaultDuration(kind: NotificationKind): number {
  // hermes parity: errors/warnings stay until dismissed.
  return kind === "error" || kind === "warning" ? 0 : 5000;
}

function defaultPlacement(kind: NotificationKind, action?: NotificationAction): NotificationPlacement {
  return kind === "error" || kind === "warning" || action ? "default" : "bottom-right";
}

function cleanErrorText(value: string): string {
  return value.replace(/^Error:\s*/, "").trim();
}

function ensureRoots(): { center: HTMLElement; corner: HTMLElement } {
  let center = document.getElementById("notify-center");
  if (!center) {
    center = document.createElement("div");
    center.id = "notify-center";
    center.className = "notify-region notify-center";
    center.setAttribute("role", "region");
    center.setAttribute("aria-label", t.notify.stackTitle);
    document.body.appendChild(center);
  }
  let corner = document.getElementById("notify-corner");
  if (!corner) {
    corner = document.createElement("div");
    corner.id = "notify-corner";
    corner.className = "notify-region notify-corner";
    corner.setAttribute("role", "region");
    corner.setAttribute("aria-label", t.notify.stackTitle);
    document.body.appendChild(corner);
  }
  return { center, corner };
}

function itemNode(note: AppNotification): HTMLElement {
  const node = document.createElement("div");
  node.className = `notify-item notify-${note.kind}`;
  node.setAttribute("role", note.kind === "error" ? "alert" : "status");
  node.dataset.id = note.id;

  const icon = document.createElement("span");
  icon.className = "notify-icon";
  icon.textContent = KIND_ICON[note.kind];
  node.appendChild(icon);

  const body = document.createElement("div");
  body.className = "notify-body";
  if (note.title) {
    const title = document.createElement("div");
    title.className = "notify-title";
    title.textContent = note.title;
    body.appendChild(title);
  }
  const message = document.createElement("div");
  message.className = "notify-message";
  message.textContent = note.kind === "error" ? cleanErrorText(note.message) : note.message;
  body.appendChild(message);
  if (note.meta) {
    const meta = document.createElement("div");
    meta.className = "notify-meta";
    meta.textContent = note.meta;
    body.appendChild(meta);
  }
  if (note.detail && note.detail !== note.message) {
    const details = document.createElement("details");
    details.className = "notify-detail";
    const summary = document.createElement("summary");
    summary.textContent = t.notify.details;
    const pre = document.createElement("pre");
    pre.textContent = note.detail;
    details.append(summary, pre);
    body.appendChild(details);
  }
  if (note.action) {
    const action = document.createElement("button");
    action.className = "notify-action";
    action.type = "button";
    action.textContent = note.action.label;
    action.onclick = () => {
      note.action?.onClick();
      dismissNotification(note.id);
    };
    body.appendChild(action);
  }
  node.appendChild(body);

  const close = document.createElement("button");
  close.className = "notify-close";
  close.type = "button";
  close.setAttribute("aria-label", t.notify.dismiss);
  close.textContent = "✕";
  close.onclick = () => dismissNotification(note.id);
  node.appendChild(close);
  return node;
}

function render(): void {
  const { center, corner } = ensureRoots();
  center.innerHTML = "";
  corner.innerHTML = "";

  const centerItems = items.filter((n) => n.placement === "default");
  const cornerItems = items.filter((n) => n.placement === "bottom-right");

  if (centerItems.length > 0) {
    const [latest, ...older] = centerItems;
    center.appendChild(itemNode(latest));
    if (expanded) {
      for (const note of older) center.appendChild(itemNode(note));
    }
    if (older.length > 0) {
      const bar = document.createElement("div");
      bar.className = "notify-stackbar";
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.textContent = expanded ? `Hide ${older.length} more` : `Show ${older.length} more`;
      toggle.onclick = () => {
        expanded = !expanded;
        render();
      };
      const clear = document.createElement("button");
      clear.type = "button";
      clear.textContent = t.notify.clearAll;
      clear.onclick = () => clearNotifications();
      bar.append(toggle, clear);
      center.appendChild(bar);
    }
  }

  for (const note of cornerItems.slice(-AMBIENT_CAP)) {
    corner.appendChild(itemNode(note));
  }
}

function scheduleDismiss(note: AppNotification, durationMs: number): void {
  if (durationMs <= 0) return;
  const handle = window.setTimeout(() => dismissNotification(note.id), durationMs);
  timers.set(note.id, handle);
}

/** Publish one notification (hermes `notify`). */
export function notify(input: NotificationInput): string {
  const kind = input.kind ?? "info";
  const note: AppNotification = {
    ...input,
    id: input.id ?? `n${++counter}`,
    kind,
    createdAt: Date.now(),
    placement: input.placement ?? defaultPlacement(kind, input.action),
  };
  // Dedupe by explicit id: replace in place.
  const existing = items.findIndex((n) => n.id === note.id);
  if (existing >= 0) {
    const old = timers.get(note.id);
    if (old) window.clearTimeout(old);
    timers.delete(note.id);
    items.splice(existing, 1, note);
  } else {
    items.unshift(note);
  }
  scheduleDismiss(note, input.durationMs ?? defaultDuration(kind));
  render();
  return note.id;
}

/** Convenience: sticky error toast (hermes `notifyError`). */
export function notifyError(message: string, detail?: string): string {
  return notify({ kind: "error", message, detail });
}

/** Convenience: quiet ambient success toast. */
export function notifySuccess(message: string): string {
  return notify({ kind: "success", message });
}

export function dismissNotification(id: string): void {
  const index = items.findIndex((n) => n.id === id);
  if (index < 0) return;
  const handle = timers.get(id);
  if (handle) window.clearTimeout(handle);
  timers.delete(id);
  items.splice(index, 1);
  if (items.filter((n) => n.placement === "default").length <= 1) expanded = false;
  render();
}

export function clearNotifications(): void {
  for (const handle of timers.values()) window.clearTimeout(handle);
  timers.clear();
  items.length = 0;
  expanded = false;
  render();
}
