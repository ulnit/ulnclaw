// Boot-failure overlay (P253) — scoped port of hermes apps/desktop
// `components/boot-failure-overlay.tsx`: when the cold-boot health poll
// exhausts, show a structured recovery card (title, description,
// Retry / Open settings / Dismiss) instead of a bare toast. hermes'
// remote re-auth, installer-repair and sign-in variants don't apply to
// the ulnclaw shell (local gateway only), so the action set is the
// local-recovery subset. Retry re-runs the connecting-overlay poll
// cycle; the overlay never resurrects once a healthy connection lands.

import { t } from "./i18n";

let overlay: HTMLDivElement | null = null;
let resolved = false;

function mount(onRetry: () => void, onOpenSettings: () => void): HTMLDivElement {
  const node = document.createElement("div");
  node.id = "boot-failure-overlay";
  node.hidden = true;
  const card = document.createElement("div");
  card.className = "boot-failure-card";
  const glyph = document.createElement("div");
  glyph.className = "boot-failure-glyph";
  glyph.setAttribute("aria-hidden", "true");
  glyph.textContent = "⚠️";
  const title = document.createElement("h2");
  title.className = "boot-failure-title";
  title.textContent = t.boot.failureTitle;
  const description = document.createElement("p");
  description.className = "boot-failure-description";
  description.textContent = t.boot.unreachableDetail;
  const actions = document.createElement("div");
  actions.className = "boot-failure-actions";
  const retry = document.createElement("button");
  retry.className = "primary";
  retry.type = "button";
  retry.textContent = t.boot.retry;
  retry.onclick = () => {
    node.hidden = true;
    onRetry();
  };
  const settings = document.createElement("button");
  settings.className = "ghost";
  settings.type = "button";
  settings.textContent = t.boot.openSettings;
  settings.onclick = () => {
    node.hidden = true;
    onOpenSettings();
  };
  const dismiss = document.createElement("button");
  dismiss.className = "ghost";
  dismiss.type = "button";
  dismiss.textContent = t.boot.dismiss;
  dismiss.onclick = () => {
    node.hidden = true;
  };
  actions.append(retry, settings, dismiss);
  card.append(glyph, title, description, actions);
  node.appendChild(card);
  document.body.appendChild(node);
  return node;
}

/** Show the recovery card (ignored once the boot has succeeded). */
export function showBootFailure(onRetry: () => void, onOpenSettings: () => void): void {
  if (resolved) return;
  if (!overlay) overlay = mount(onRetry, onOpenSettings);
  overlay.hidden = false;
}

/** Hide the card and latch — a healthy gateway means no failure UI. */
export function resolveBootFailure(): void {
  resolved = true;
  if (overlay) overlay.hidden = true;
}
