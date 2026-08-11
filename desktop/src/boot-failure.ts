// Boot-failure overlay (P253) — scoped port of hermes apps/desktop
// `components/boot-failure-overlay.tsx`: when the cold-boot health poll
// exhausts, show a structured recovery card (title, description,
// Retry / Open settings / Dismiss) instead of a bare toast. hermes'
// remote re-auth, installer-repair and sign-in variants don't apply to
// the ulnclaw shell (local gateway only), so the action set is the
// local-recovery subset. Retry re-runs the connecting-overlay poll
// cycle; the overlay never resurrects once a healthy connection lands.

import { t } from "./i18n";

/** P806: shell-side boot diagnostics snapshot (desktop_gateway_diagnostics). */
export interface BootDiagnostics {
  binary: string | null;
  bundled: string | null;
  port: number;
  log_path: string | null;
  child_pid: number | null;
  child_alive: boolean;
  log_tail: string;
  spawnError?: string | null;
}

type DiagnosticsLoader = () => Promise<BootDiagnostics | null>;

let overlay: HTMLDivElement | null = null;
let resolved = false;
let loader: DiagnosticsLoader | null = null;

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
  const diagnostics = document.createElement("details");
  diagnostics.className = "boot-failure-diagnostics";
  const summary = document.createElement("summary");
  summary.textContent = t.boot.diagTitle;
  const body = document.createElement("div");
  body.className = "boot-failure-diagnostics-body";
  diagnostics.append(summary, body);
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
  card.append(glyph, title, description, diagnostics, actions);
  node.appendChild(card);
  document.body.appendChild(node);
  return node;
}

/** Render the P806 diagnostics snapshot into the card's <details>. */
function renderDiagnostics(diag: BootDiagnostics | null): void {
  const body = overlay?.querySelector(".boot-failure-diagnostics-body");
  if (!body) return;
  body.textContent = "";
  const addLine = (label: string, value: string, bad = false): void => {
    const row = document.createElement("div");
    row.className = "boot-failure-diag-row";
    const key = document.createElement("span");
    key.className = "boot-failure-diag-key";
    key.textContent = label;
    const val = document.createElement("span");
    if (bad) val.className = "boot-failure-diag-bad";
    val.textContent = value;
    row.append(key, val);
    body.appendChild(row);
  };
  if (!diag) {
    addLine(t.boot.diagProcess, t.boot.diagNoData);
    return;
  }
  if (diag.spawnError) {
    addLine(t.boot.diagProcess, t.boot.spawnFailed.replace("{error}", diag.spawnError), true);
  }
  if (diag.binary) {
    addLine(t.boot.diagBinary, diag.binary);
  } else {
    addLine(t.boot.diagBinary, t.boot.diagNoBinary, true);
  }
  addLine(t.boot.diagPort, String(diag.port));
  addLine(
    t.boot.diagProcess,
    diag.child_pid === null
      ? t.boot.diagDead
      : `${t.boot.diagPid} ${diag.child_pid} — ${diag.child_alive ? t.boot.diagAlive : t.boot.diagDead}`,
    !diag.child_alive,
  );
  if (diag.log_path) {
    addLine(t.boot.diagLog, diag.log_path);
  }
  if (diag.log_tail.trim()) {
    const pre = document.createElement("pre");
    pre.className = "boot-failure-diag-log";
    pre.textContent = diag.log_tail;
    body.appendChild(pre);
  } else if (diag.log_path) {
    addLine("", t.boot.diagNoLog);
  }
}

async function refreshDiagnostics(): Promise<void> {
  if (!loader) return;
  try {
    renderDiagnostics(await loader());
  } catch {
    renderDiagnostics(null);
  }
}

/** Show the recovery card (ignored once the boot has succeeded). */
export function showBootFailure(
  onRetry: () => void,
  onOpenSettings: () => void,
  diagnostics?: DiagnosticsLoader,
): void {
  if (resolved) return;
  if (!overlay) overlay = mount(onRetry, onOpenSettings);
  if (diagnostics) loader = diagnostics;
  overlay.hidden = false;
  void refreshDiagnostics();
}

/** Hide the card and latch — a healthy gateway means no failure UI. */
export function resolveBootFailure(): void {
  resolved = true;
  if (overlay) overlay.hidden = true;
}
