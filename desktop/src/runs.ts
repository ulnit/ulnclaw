// Runs view — tracked async runs (`/v1/runs`) with live approval
// resolution: waiting runs surface their pending command with
// once/session/always/deny buttons (gateway approval gateway parity with
// the terminal approve prompt), plus stop and result inspection.

import type { GatewayClient, RunRow, DelegationRow } from "./gateway";
import { t } from "./i18n";

const REFRESH_MS = 5_000;
const STATUS_FILTER_KEY = "ulnclaw.runs.statusFilter";
const DELEGATION_FILTER_KEY = "ulnclaw.runs.delegationFilter";

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
  /** Live status timeline per run, fed by /v1/runs/:id/events (P322). */
  private timelines = new Map<string, { status: string; at: number }[]>();
  private subs = new Map<string, AbortController>();
  /** Cached delegations so the P505 filter re-renders without a refetch. */
  private lastDelegations: DelegationRow[] = [];
  /** P539: cached runs + live text filter over message/id/session. */
  private lastRuns: RunRow[] = [];
  private textFilter = "";

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
    // P425: jump from a run card into its session in the chat view.
    private openSession: ((sessionId: string) => void) | null = null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="runs-header">
        <span id="runs-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <input id="runs-filter" type="search" data-i18n-ph="runs.filterPlaceholder" />
        <select id="runs-status-filter" data-i18n-title="runs.statusFilterTitle">
          <option value="all" data-i18n="runs.statusAll">All statuses</option>
          <option value="running" data-i18n="runs.statusRunning">Running</option>
          <option value="queued" data-i18n="runs.statusQueued">Queued</option>
          <option value="waiting_for_approval" data-i18n="runs.statusWaiting">Waiting for approval</option>
          <option value="completed" data-i18n="runs.statusCompleted">Completed</option>
          <option value="failed" data-i18n="runs.statusFailed">Failed</option>
          <option value="stopped" data-i18n="runs.statusStopped">Stopped</option>
        </select>
        <button id="runs-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="runs-status" class="config-status" hidden></div>
      <div id="runs-list" class="runs-list"></div>
      <h3 class="config-section runs-delegations-title" data-i18n="runs.delegationsTitle">Delegations</h3>
      <select id="runs-delegations-filter" data-i18n-title="runs.delegationsFilterTitle">
        <option value="all" data-i18n="runs.statusAll">All statuses</option>
        <option value="running" data-i18n="runs.statusRunning">Running</option>
        <option value="completed" data-i18n="runs.statusCompleted">Completed</option>
        <option value="failed" data-i18n="runs.statusFailed">Failed</option>
      </select>
      <div id="delegations-list" class="runs-list"></div>
    `;
    this.root.querySelector("#runs-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    const statusSelect = this.root.querySelector("#runs-status-filter") as HTMLSelectElement;
    statusSelect.value = this.statusFilter;
    statusSelect.addEventListener("change", () => {
      window.localStorage.setItem(STATUS_FILTER_KEY, statusSelect.value);
      this.refresh().catch(() => undefined);
    });
    const delegationSelect = this.root.querySelector("#runs-delegations-filter") as HTMLSelectElement;
    delegationSelect.value = this.delegationFilter;
    delegationSelect.addEventListener("change", () => {
      window.localStorage.setItem(DELEGATION_FILTER_KEY, delegationSelect.value);
      this.renderDelegations(this.lastDelegations);
    });
    // P539: live text filter — re-renders the cached rows client-side.
    this.root.querySelector("#runs-filter")!.addEventListener("input", () => {
      this.textFilter = (
        (this.root.querySelector("#runs-filter") as HTMLInputElement).value || ""
      )
        .trim()
        .toLowerCase();
      this.render(this.lastRuns);
      this.renderDelegations(this.lastDelegations);
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
    for (const controller of this.subs.values()) controller.abort();
    this.subs.clear();
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#runs-status") as HTMLElement;
    el.hidden = !message;
    el.textContent = message;
    el.classList.toggle("error", isError);
  }

  /** Persisted run-status filter (P502). */
  private get statusFilter(): string {
    return window.localStorage.getItem(STATUS_FILTER_KEY) ?? "all";
  }

  /** Persisted delegation-status filter (P505). */
  private get delegationFilter(): string {
    return window.localStorage.getItem(DELEGATION_FILTER_KEY) ?? "all";
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
    this.lastRuns = runs;
    this.reconcileSubscriptions(runs);
    const list = this.root.querySelector("#runs-list") as HTMLElement;
    list.innerHTML = "";
    const filter = this.statusFilter;
    // P539: status select first, then the live text filter (message,
    // run id, or session id).
    const query = this.textFilter;
    const visible = (
      filter === "all" ? runs : runs.filter((run) => run.status === filter)
    ).filter(
      (run) =>
        query === "" ||
        run.message.toLowerCase().includes(query) ||
        run.run_id.toLowerCase().includes(query) ||
        (run.session_id || "").toLowerCase().includes(query),
    );
    if (visible.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = runs.length === 0 ? t.runs.empty : t.runs.filteredEmpty;
      list.appendChild(empty);
      return;
    }
    for (const run of visible) {
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
          ${run.status === "completed" || run.status === "failed"
            ? `<button class="ghost run-rerun" title="${escapeHtml(t.runs.rerunTitle)}">${escapeHtml(t.runs.rerun)}</button>`
            : ""}
        </div>
        <div class="run-message">${escapeHtml(run.message)}</div>
        ${run.error ? `<div class="run-error">${escapeHtml(run.error)}</div>` : ""}
        ${run.result ? `<details class="run-result"><summary>${escapeHtml(t.runs.result)}<button class="ghost run-copy-result">${escapeHtml(t.runs.copyResult)}</button></summary><pre>${escapeHtml(run.result)}</pre></details>` : ""}
        ${run.session_id ? `<div class="run-session">session: <button class="ghost run-session-link" title="${escapeHtml(t.runs.openSessionTitle)}"><code>${escapeHtml(run.session_id.slice(0, 12))}</code></button></div>` : ""}
      `;
      card.dataset.runId = run.run_id;
      const chips = this.timelineChips(run.run_id);
      if (chips) {
        card.insertAdjacentHTML(
          "beforeend",
          `<div class="run-timeline" title="${escapeHtml(t.runs.timelineTitle)}">${chips}</div>`,
        );
      }
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
      // P572: settled runs can be dispatched again with the same message.
      const rerunBtn = card.querySelector(".run-rerun");
      if (rerunBtn) {
        rerunBtn.addEventListener("click", () => {
          this.rerunRun(run).catch(() => undefined);
        });
      }
      const sessionLink = card.querySelector(".run-session-link");
      if (sessionLink && run.session_id && this.openSession) {
        const target = run.session_id;
        sessionLink.addEventListener("click", () => this.openSession!(target));
      }
      const copyBtn = card.querySelector<HTMLButtonElement>(".run-copy-result");
      if (copyBtn && run.result) {
        const result = run.result;
        copyBtn.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          void navigator.clipboard.writeText(result).then(
            () => this.flashCopy(copyBtn, true),
            () => this.flashCopy(copyBtn, false),
          );
        });
      }
      list.appendChild(card);
    }
  }

  /** P504: transient copied/failed feedback on the run-result copy button. */
  private flashCopy(button: HTMLButtonElement, ok: boolean): void {
    const original = button.textContent ?? "";
    button.textContent = ok ? t.runs.copiedResult : t.runs.copyFailed;
    window.setTimeout(() => {
      button.textContent = original;
    }, 1200);
  }

  /** Keep one SSE subscription per active run; drop stale ones (P322). */
  private reconcileSubscriptions(runs: RunRow[]): void {
    const seen = new Set(runs.map((run) => run.run_id));
    for (const [runId, controller] of this.subs) {
      if (!seen.has(runId)) {
        controller.abort();
        this.subs.delete(runId);
        this.timelines.delete(runId);
      }
    }
    for (const run of runs) {
      if (["running", "queued", "waiting_for_approval"].includes(run.status)) {
        this.recordTransition(run.run_id, run.status);
        this.subscribe(run.run_id);
      }
    }
  }

  private subscribe(runId: string): void {
    if (this.subs.has(runId)) return;
    const client = this.client();
    if (!client) return;
    const controller = new AbortController();
    this.subs.set(runId, controller);
    client
      .runEvents(runId, (_name, run) => {
        this.recordTransition(run.run_id, run.status);
      }, controller.signal)
      .catch(() => undefined)
      .finally(() => {
        this.subs.delete(runId);
      });
  }

  private recordTransition(runId: string, status: string): void {
    const timeline = this.timelines.get(runId) ?? [];
    const last = timeline[timeline.length - 1];
    if (last && last.status === status) return;
    timeline.push({ status, at: Date.now() / 1000 });
    if (timeline.length > 24) timeline.shift();
    this.timelines.set(runId, timeline);
    this.renderTimeline(runId);
  }

  private timelineChips(runId: string): string {
    const timeline = this.timelines.get(runId) ?? [];
    return timeline
      .map((entry) => {
        const when = new Date(entry.at * 1000).toLocaleTimeString();
        return `<span class="run-timeline-chip">${escapeHtml(entry.status)} \u00b7 ${escapeHtml(when)}</span>`;
      })
      .join("");
  }

  private renderTimeline(runId: string): void {
    const card = this.root.querySelector(`.run-card[data-run-id="${CSS.escape(runId)}"]`);
    if (!card) return;
    let timeline = card.querySelector(".run-timeline");
    if (!timeline) {
      timeline = document.createElement("div");
      timeline.className = "run-timeline";
      timeline.setAttribute("title", t.runs.timelineTitle);
      card.appendChild(timeline);
    }
    timeline.innerHTML = this.timelineChips(runId);
  }

  private renderDelegations(delegations: DelegationRow[]): void {
    this.lastDelegations = delegations;
    const list = this.root.querySelector("#delegations-list") as HTMLElement;
    list.innerHTML = "";
    const filter = this.delegationFilter;
    // P539: the same text filter narrows delegations by parent session
    // key, log dir, or delegation id.
    const query = this.textFilter;
    const visible = (
      filter === "all"
        ? delegations
        : delegations.filter((delegation) => delegation.status === filter)
    ).filter(
      (delegation) =>
        query === "" ||
        delegation.parent_session_key.toLowerCase().includes(query) ||
        delegation.log_dir.toLowerCase().includes(query) ||
        delegation.id.toLowerCase().includes(query),
    );
    if (visible.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent =
        delegations.length === 0 ? t.runs.noDelegations : t.runs.delegationsFilteredEmpty;
      list.appendChild(empty);
      return;
    }
    for (const delegation of visible) {
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
    if (!window.confirm(t.runs.stopConfirm.replace("{id}", runId.slice(0, 8)))) return;
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

  /** P572: dispatch a fresh run with the same message (and session). */
  private async rerunRun(run: RunRow): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    try {
      const runId = await client.runStart(run.message, run.session_id ?? undefined);
      if (!runId) throw new Error("gateway rejected the run");
      await this.refresh();
    } catch (error) {
      this.status(
        t.runs.rerunFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
    }
  }
}
