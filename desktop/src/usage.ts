// Usage view — token-accounting dashboard over the gateway `/api/usage`
// API (same metrics the `ulnclaw status` CLI prints): process/store
// summary cards plus a per-session token table with proportional bars.

import type { GatewayClient, UsagePayload, UsageSessionRow, InsightsReport } from "./gateway";
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
      <h3 class="usage-section-title" data-i18n="insights.title">Insights</h3>
      <div id="insights-controls" class="insights-controls">
        <select id="insights-days">
          <option value="7" data-i18n="insights.days7">Last 7 days</option>
          <option value="30" selected data-i18n="insights.days30">Last 30 days</option>
          <option value="90" data-i18n="insights.days90">Last 90 days</option>
        </select>
        <span id="insights-status" class="config-note"></span>
      </div>
      <div id="insights-body"></div>
    `;
    this.root.querySelector("#usage-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
      this.refreshInsights().catch(() => undefined);
    });
    (this.root.querySelector("#insights-days") as HTMLSelectElement).addEventListener("change", () => {
      this.refreshInsights().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
    this.refreshInsights().catch(() => undefined);
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

  private async refreshInsights(): Promise<void> {
    const client = this.client();
    const body = this.root.querySelector("#insights-body") as HTMLElement;
    const status = this.root.querySelector("#insights-status") as HTMLElement;
    if (!client) return;
    const days = Number((this.root.querySelector("#insights-days") as HTMLSelectElement).value) || 30;
    try {
      const report = await client.insights(days);
      this.renderInsights(report);
      status.textContent = "";
    } catch (error) {
      body.innerHTML = "";
      status.textContent = t.insights.loadFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private renderInsights(report: InsightsReport): void {
    const body = this.root.querySelector("#insights-body") as HTMLElement;
    const u = t.insights;
    if (report.empty) {
      body.innerHTML = `<p class="empty">${escapeHtml(u.empty)}</p>`;
      return;
    }
    const o = report.overview;
    const cost = o.cost_known ? `$${o.estimated_cost_usd.toFixed(2)}` : "—";
    const avgMin = Math.round(o.avg_session_seconds / 60);
    const cards = [
      this.card(u.sessions, String(o.total_sessions), `${u.activeDays} ${o.active_days}`),
      this.card(u.messages, String(o.total_messages)),
      this.card(u.toolCalls, String(o.total_tool_calls)),
      this.card(u.tokens, fmtTokens(o.total_tokens), `${fmtTokens(o.input_tokens)} ${t.usage.input} · ${fmtTokens(o.output_tokens)} ${t.usage.output}`),
      this.card(u.estCost, cost),
      this.card(u.avgSession, `${avgMin} min`),
    ].join("");

    const modelRows = report.models
      .slice(0, 8)
      .map(
        (m) => `<tr>
          <td>${escapeHtml(m.model)}</td>
          <td class="num">${m.sessions}</td>
          <td class="num">${fmtTokens(m.total_tokens)}</td>
        </tr>`,
      )
      .join("");
    const toolRows = report.tools
      .slice(0, 8)
      .map(
        (tool) => `<tr>
          <td>${escapeHtml(tool.tool)}</td>
          <td class="num">${tool.calls}</td>
        </tr>`,
      )
      .join("");
    const sessionRows = report.top_sessions
      .slice(0, 8)
      .map(
        (session) => `<tr>
          <td>${escapeHtml(session.title || session.id.slice(0, 8))}</td>
          <td class="num">${session.messages}</td>
          <td class="num">${fmtTokens(session.total_tokens)}</td>
        </tr>`,
      )
      .join("");

    body.innerHTML = `
      <div class="usage-cards">${cards}</div>
      <div class="insights-tables">
        <div>
          <h4 class="config-section">${escapeHtml(u.topModels)}</h4>
          ${modelRows ? `<table class="usage-table"><thead><tr><th>${escapeHtml(u.colModel)}</th><th class="num">${escapeHtml(u.sessions)}</th><th class="num">${escapeHtml(u.tokens)}</th></tr></thead><tbody>${modelRows}</tbody></table>` : `<p class="empty">${escapeHtml(u.empty)}</p>`}
        </div>
        <div>
          <h4 class="config-section">${escapeHtml(u.topTools)}</h4>
          ${toolRows ? `<table class="usage-table"><thead><tr><th>${escapeHtml(u.colTool)}</th><th class="num">${escapeHtml(u.calls)}</th></tr></thead><tbody>${toolRows}</tbody></table>` : `<p class="empty">${escapeHtml(u.empty)}</p>`}
        </div>
        <div>
          <h4 class="config-section">${escapeHtml(u.topSessions)}</h4>
          ${sessionRows ? `<table class="usage-table"><thead><tr><th>${escapeHtml(u.colSession)}</th><th class="num">${escapeHtml(u.messages)}</th><th class="num">${escapeHtml(u.tokens)}</th></tr></thead><tbody>${sessionRows}</tbody></table>` : `<p class="empty">${escapeHtml(u.empty)}</p>`}
        </div>
      </div>
    `;
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
