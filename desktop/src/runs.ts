// Runs view — tracked async runs (`/v1/runs`) with live approval
// resolution: waiting runs surface their pending command with
// once/session/always/deny buttons (gateway approval gateway parity with
// the terminal approve prompt), plus stop and result inspection.

import type { GatewayClient, RunRow, DelegationRow } from "./gateway";
import { t } from "./i18n";

const REFRESH_MS = 5_000;

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fmtWhen(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString();
}

const STATUS_CLASS: Record<string, string> = {
  running: "run-running",
  queued: "run-queued",
  waiting_for_approval: "run-waiting",
  completed: "run-completed",
  failed: "run-failed",
  stopped: "run-stopped",
};

export class RunsWidget {
  private timer: number | null = null;
  private busy = false;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="runs-header">
        <span id="runs-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="runs-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="runs-status" class="config-status" hidden></div>
      <div id="runs-list" class="runs-list"></div>
      <h3 class="config-section runs-delegations-title" data-i18n="runs.delegationsTitle">Delegations</h3>
      <div id="delegations-list" class="runs-list"></div>
    `;
    this.root.querySelector("#runs-refresh")!.addEventListener("click", () => {
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

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#runs-status") as HTMLElement;
    el.hidden = !message;
    el.textContent = message;
    el.classList.toggle("error", isError);
  }

  async refresh(): Promise<void> {
    const client = this.client();
    if (!client) {
      this.status(t.config.notConnected, true);
      return;
    }
    try {
      const [runs, delegations] = await Promise.all([
        client.listRuns(),
        client.listDelegations().catch(() => [] as DelegationRow[]),
      ]);
      this.render(runs);
      this.renderDelegations(delegations);
      const active = runs.filter((run) =>
        ["running", "queued", "waiting_for_approval"].includes(run.status),
      ).length;
      (this.root.querySelector("#runs-count") as HTMLElement).textContent =
        t.runs.count.replace("{count}", String(runs.length)).replace("{active}", String(active));
      this.status("");
    } catch (error) {
      this.status(
        t.runs.loadFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    }
  }

  private render(runs: RunRow[]): void {
    const list = this.root.querySelector("#runs-list") as HTMLElement;
    list.innerHTML = "";
    if (runs.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.runs.empty;
      list.appendChild(empty);
      return;
    }
    for (const run of runs) {
      const card = document.createElement("div");
      card.className = `run-card ${STATUS_CLASS[run.status] || ""}`;
      const stopping = run.stop_requested && run.status === "running";
      card.innerHTML = `
        <div class="run-head">
          <span class="run-status">${escapeHtml(run.status)}</span>
          <span class="run-id">${escapeHtml(run.run_id.slice(0, 8))}</span>
          ${run.iterations != null ? `<span class="run-iter">${run.iterations} iter</span>` : ""}
          <span class="spacer"></span>
          <span class="run-when">${escapeHtml(fmtWhen(run.created_at))}</span>
          ${run.status === "running" || run.status === "queued"
            ? `<button class="ghost danger run-stop">${escapeHtml(stopping ? t.runs.stopping : t.runs.stop)}</button>`
            : ""}
        </div>
        <div class="run-message">${escapeHtml(run.message)}</div>
        ${run.error ? `<div class="run-error">${escapeHtml(run.error)}</div>` : ""}
        ${run.result ? `<details class="run-result"><summary>${escapeHtml(t.runs.result)}</summary><pre>${escapeHtml(run.result)}</pre></details>` : ""}
        ${run.session_id ? `<div class="run-session">session: <code>${escapeHtml(run.session_id.slice(0, 12))}</code></div>` : ""}
      `;
      if (run.status === "waiting_for_approval" && run.approval && !run.approval.resolved) {
        const approval = document.createElement("div");
        approval.className = "run-approval";
        approval.innerHTML = `
          <div class="run-approval-title">${escapeHtml(t.runs.approvalTitle)}</div>
          <pre class="run-approval-command">${escapeHtml(run.approval.command)}</pre>
          ${run.approval.reason ? `<div class="run-approval-reason">${escapeHtml(run.approval.reason)}</div>` : ""}
          <div class="run-approval-actions">
            <button class="primary" data-decision="once">${escapeHtml(t.runs.approveOnce)}</button>
            <button class="ghost" data-decision="session">${escapeHtml(t.runs.approveSession)}</button>
            <button class="ghost" data-decision="always">${escapeHtml(t.runs.approveAlways)}</button>
            <button class="danger" data-decision="deny">${escapeHtml(t.runs.deny)}</button>
          </div>
        `;
        approval.querySelectorAll<HTMLButtonElement>("[data-decision]").forEach((button) => {
          button.addEventListener("click", () => {
            const decision = button.dataset.decision as "once" | "session" | "always" | "deny";
            this.approve(run.run_id, decision).catch(() => undefined);
          });
        });
        card.appendChild(approval);
      } else if (run.approval?.resolved) {
        const note = document.createElement("div");
        note.className = "run-approval-resolved";
        note.textContent = `${t.runs.approvalTitle}: ${run.approval.resolved}`;
        card.appendChild(note);
      }
      const stopBtn = card.querySelector(".run-stop");
      if (stopBtn) {
        stopBtn.addEventListener("click", () => {
          this.stopRun(run.run_id).catch(() => undefined);
        });
      }
      list.appendChild(card);
    }
  }

  private renderDelegations(delegations: DelegationRow[]): void {
    const list = this.root.querySelector("#delegations-list") as HTMLElement;
    list.innerHTML = "";
    if (delegations.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.runs.noDelegations;
      list.appendChild(empty);
      return;
    }
    for (const delegation of delegations) {
      const card = document.createElement("div");
      card.className = `run-card ${STATUS_CLASS[delegation.status] || ""}`;
      const when = new Date(delegation.created_ms).toLocaleString();
      card.innerHTML = `
        <div class="run-head">
          <span class="run-status">${escapeHtml(delegation.status)}</span>
          <span class="run-id">${escapeHtml(delegation.id.slice(0, 8))}</span>
          ${delegation.tasks != null ? `<span class="run-iter">${delegation.tasks} tasks</span>` : ""}
          <span class="spacer"></span>
          <span class="run-when">${escapeHtml(when)}</span>
        </div>
        <div class="run-session">parent: <code>${escapeHtml(delegation.parent_session_key)}</code></div>
        ${
          delegation.status === "completed"
            ? `<details class="run-result delegation-result"><summary>${escapeHtml(t.runs.result)}</summary><div class="delegation-body">${escapeHtml(t.runs.loading)}</div></details>`
            : ""
        }
      `;
      const details = card.querySelector<HTMLDetailsElement>(".delegation-result");
      if (details) {
        let loaded = false;
        details.addEventListener("toggle", () => {
          if (details.open && !loaded) {
            loaded = true;
            this.loadDelegationResult(delegation.id, details).catch(() => undefined);
          }
        });
      }
      list.appendChild(card);
    }
  }

  private async loadDelegationResult(id: string, details: HTMLDetailsElement): Promise<void> {
    const client = this.client();
    const body = details.querySelector(".delegation-body") as HTMLElement;
    if (!client) return;
    try {
      const detail = await client.delegationDetail(id);
      const result = detail.result;
      let text: string;
      if (result === null || result === undefined) {
        text = t.runs.noResult;
      } else if (typeof result === "object" && result !== null && "report" in result) {
        text = String((result as { report: unknown }).report ?? JSON.stringify(result, null, 2));
      } else if (typeof result === "string") {
        text = result;
      } else {
        text = JSON.stringify(result, null, 2);
      }
      const pre = document.createElement("pre");
      pre.textContent = text;
      body.replaceChildren(pre);
    } catch (error) {
      body.textContent = t.runs.loadFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private async approve(runId: string, decision: "once" | "session" | "always" | "deny"): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    try {
      await client.approveRun(runId, decision);
      await this.refresh();
    } catch (error) {
      this.status(
        t.runs.approveFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
    }
  }

  private async stopRun(runId: string): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    try {
      await client.stopRun(runId);
      await this.refresh();
    } catch (error) {
      this.status(
        t.runs.stopFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
    }
  }
}
