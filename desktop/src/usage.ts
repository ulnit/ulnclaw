// Usage view — token-accounting dashboard over the gateway `/api/usage`
// API (same metrics the `ulnclaw status` CLI prints): process/store
// summary cards plus a per-session token table with proportional bars.

import type { GatewayClient, UsagePayload, UsageSessionRow } from "./gateway";
import { fmt, t } from "./i18n";

const REFRESH_MS = 10_000;

/** Compact token count: 999 → "999", 12 345 → "12.3k", 3 400 000 → "3.4M". */
export function fmtTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0";
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

function fmtWhen(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString();
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class UsageWidget {
  private timer: number | null = null;
  private busy = false;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="usage-header">
        <span id="usage-window" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="usage-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="usage-cards" class="usage-cards"></div>
      <h3 class="usage-section-title" data-i18n="usage.perSession">Per-session breakdown</h3>
      <div id="usage-sessions"></div>
    `;
    this.root.querySelector("#usage-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
    this.timer = window.setInterval(() => {
      this.refresh().catch(() => undefined);
    }, REFRESH_MS);
  }

  stop(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
  }

  async refresh(): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    try {
      const usage = await client.usage(50);
      this.render(usage);
    } catch {
      // Gateway offline or still booting — leave last render in place.
    } finally {
      this.busy = false;
    }
  }

  private card(label: string, value: string, detail = ""): string {
    return `
      <div class="usage-card">
        <div class="usage-card-value">${escapeHtml(value)}</div>
        <div class="usage-card-label">${escapeHtml(label)}</div>
        ${detail ? `<div class="usage-card-detail">${escapeHtml(detail)}</div>` : ""}
      </div>`;
  }

  private render(usage: UsagePayload): void {
    const u = t.usage;
    const proc = usage.process;
    const store = usage.store;

    this.root.querySelector("#usage-window")!.textContent = fmt(u.windowNote, {
      count: String(usage.sessions.length),
    });

    const cards = this.root.querySelector("#usage-cards")!;
    cards.innerHTML = [
      this.card(u.totalTokens, fmtTokens(store.total_tokens), `${u.input} ${fmtTokens(store.input_tokens)} · ${u.output} ${fmtTokens(store.output_tokens)}`),
      this.card(u.sessions, String(store.sessions), `${u.messages} ${store.messages}`),
      this.card(u.processTokens, fmtTokens(proc.total_tokens), `${u.prompt} ${fmtTokens(proc.prompt_tokens)} · ${u.completion} ${fmtTokens(proc.completion_tokens)}`),
      this.card(u.toolCalls, String(proc.tool_calls)),
      this.card(u.requests, String(proc.requests.chat_completions + proc.requests.responses + proc.requests.session_chats),
        `chat ${proc.requests.chat_completions} · responses ${proc.requests.responses} · session ${proc.requests.session_chats}`),
      this.card(u.runs, String(proc.runs.started), `${u.completed} ${proc.runs.completed} · ${u.failed} ${proc.runs.failed}`),
    ].join("");

    const list = this.root.querySelector("#usage-sessions")!;
    if (usage.sessions.length === 0) {
      list.innerHTML = `<p class="empty" data-i18n="usage.empty">${escapeHtml(u.empty)}</p>`;
      return;
    }
    const max = Math.max(1, ...usage.sessions.map((s) => s.total_tokens));
    const rows = usage.sessions
      .map((s) => this.sessionRow(s, max))
      .join("");
    list.innerHTML = `
      <table class="usage-table">
        <thead>
          <tr>
            <th>${escapeHtml(u.colSession)}</th>
            <th>${escapeHtml(u.colModel)}</th>
            <th class="num">${escapeHtml(u.colMessages)}</th>
            <th class="num">${escapeHtml(u.colInput)}</th>
            <th class="num">${escapeHtml(u.colOutput)}</th>
            <th class="num">${escapeHtml(u.colTotal)}</th>
            <th>${escapeHtml(u.colStarted)}</th>
          </tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>`;
  }

  private sessionRow(row: UsageSessionRow, max: number): string {
    const u = t.usage;
    const title = row.title?.trim() || row.id.slice(0, 8);
    const pct = Math.max(2, Math.round((row.total_tokens / max) * 100));
    return `
      <tr>
        <td class="usage-session-cell">
          <span class="usage-session-title" title="${escapeHtml(row.id)}">${escapeHtml(title)}</span>
          <span class="usage-bar-track"><span class="usage-bar-fill" style="width:${pct}%"></span></span>
        </td>
        <td class="muted">${escapeHtml(row.model || "—")}</td>
        <td class="num">${row.message_count}</td>
        <td class="num">${fmtTokens(row.input_tokens)}</td>
        <td class="num">${fmtTokens(row.output_tokens)}</td>
        <td class="num usage-total">${fmtTokens(row.total_tokens)}</td>
        <td class="muted" title="${escapeHtml(fmtWhen(row.started_at))}">${escapeHtml(fmtWhen(row.started_at))}</td>
      </tr>`;
  }
}
