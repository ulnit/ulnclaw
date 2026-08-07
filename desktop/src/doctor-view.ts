// Doctor view — runs the gateway doctor checks over `GET /api/doctor`
// (the same report `ulnclaw doctor` prints) and renders ✓/⚠/✗/ℹ rows
// grouped by section, with an issues panel up top. Online provider
// probes are opt-in since they are slow.

import type { GatewayClient, DoctorCheck, McpOAuthFlow, McpServerRow, MonitoringPayload } from "./gateway";
import { t } from "./i18n";

const LEVEL_ICON: Record<DoctorCheck["level"], string> = {
  ok: "✓",
  warn: "⚠",
  fail: "✗",
  info: "ℹ",
};

const LOGS_REFRESH_MS = 10_000;
const LOGS_LINES = 150;

export class DoctorWidget {
  private busy = false;
  private logsTimer: number | null = null;
  private mcpPollers: number[] = [];

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="doctor-header">
        <button id="doctor-run" class="primary" data-i18n="doctor.run">Run doctor</button>
        <label class="check doctor-online">
          <input id="doctor-online-check" type="checkbox" />
          <span data-i18n="doctor.online">Include provider connectivity probes (slow)</span>
        </label>
        <span class="spacer"></span>
        <span id="doctor-status" class="jobs-counts"></span>
      </header>
      <div id="doctor-body" class="doctor-body"></div>
      <section id="doctor-monitoring" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="monitoring.title">Gateway monitoring</h3>
        <div id="monitoring-rows"></div>
      </section>
      <section id="doctor-browser" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="browserPanel.title">Browser (CDP)</h3>
        <div id="browser-rows"></div>
      </section>
      <section id="doctor-mcp" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="mcpPanel.title">MCP servers</h3>
        <div id="mcp-rows"></div>
      </section>
      <section id="doctor-kanban" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="kanbanPanel.title">Kanban diagnostics</h3>
        <div id="kanban-rows"></div>
      </section>
      <section id="doctor-logs" class="doctor-monitoring doctor-logs" hidden>
        <h3 class="config-section" data-i18n="logsPanel.title">Gateway log</h3>
        <div class="logs-controls">
          <select id="logs-level">
            <option value="" data-i18n="logsPanel.allLevels">All levels</option>
            <option value="INFO">INFO+</option>
            <option value="WARN">WARN+</option>
            <option value="ERROR">ERROR+</option>
          </select>
          <span id="logs-path" class="config-note"></span>
          <span class="spacer"></span>
          <button id="logs-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
        </div>
        <pre id="logs-body" class="logs-body"></pre>
      </section>
    `;
    this.root.querySelector("#doctor-run")!.addEventListener("click", () => {
      this.run().catch(() => undefined);
    });
  }

  start(): void {
    if (!this.root.querySelector(".doctor-section")) {
      this.run().catch(() => undefined);
    }
    this.loadMonitoring().catch(() => undefined);
    this.loadBrowser().catch(() => undefined);
    this.loadMcp().catch(() => undefined);
    this.loadKanban().catch(() => undefined);
    this.loadLogs().catch(() => undefined);
    if (this.logsTimer === null) {
      this.logsTimer = window.setInterval(() => {
        this.loadLogs().catch(() => undefined);
      }, LOGS_REFRESH_MS);
    }
  }

  stop(): void {
    if (this.logsTimer !== null) {
      window.clearInterval(this.logsTimer);
      this.logsTimer = null;
    }
    for (const poller of this.mcpPollers) window.clearInterval(poller);
    this.mcpPollers = [];
  }

  private status(message: string): void {
    (this.root.querySelector("#doctor-status") as HTMLElement).textContent = message;
  }

  private async run(): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    const runBtn = this.root.querySelector("#doctor-run") as HTMLButtonElement;
    runBtn.disabled = true;
    this.status(t.doctor.running);
    const online = (this.root.querySelector("#doctor-online-check") as HTMLInputElement).checked;
    const body = this.root.querySelector("#doctor-body") as HTMLElement;
    try {
      const payload = await client.doctor(online);
      this.render(payload.report.sections, payload.report.issues);
      this.status("");
    } catch (error) {
      body.innerHTML = "";
      this.status(
        t.doctor.failed.replace("{error}", error instanceof Error ? error.message : String(error)),
      );
    } finally {
      this.busy = false;
      runBtn.disabled = false;
    }
  }

  private async loadMonitoring(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-monitoring") as HTMLElement;
    const rows = this.root.querySelector("#monitoring-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const status = await client.monitoring();
      rows.innerHTML = "";
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const entries: [string, string][] = [
        [t.monitoring.healthExport, status.enabled ? on : off],
        [
          t.monitoring.metrics,
          status.metrics
            ? `${on} (${status.metrics_interval_seconds}s)`
            : off,
        ],
        [t.monitoring.diagnosticEvents, status.diagnostic_events ? on : off],
        [
          t.monitoring.warningLogs,
          status.warning_error_logs
            ? `${on} (${status.logs_interval_seconds}s)`
            : off,
        ],
        [
          t.monitoring.otlpEndpoint,
          status.otlp.endpoint
            ? `${status.otlp.endpoint} (${status.otlp.transport})`
            : t.monitoring.otlpNotConfigured,
        ],
        [t.monitoring.queueDepth, String(status.queue_depth)],
      ];
      if (status.install_id) {
        entries.push([t.monitoring.installId, status.install_id]);
      }
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      const note = document.createElement("p");
      note.className = "config-note";
      note.textContent = status.scope;
      rows.appendChild(note);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadMcp(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-mcp") as HTMLElement;
    const rows = this.root.querySelector("#mcp-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const servers = await client.mcpServers();
      rows.innerHTML = "";
      if (servers.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = t.mcpPanel.none;
        rows.appendChild(empty);
      }
      for (const server of servers) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = server.name;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        let auth: string = server.auth;
        if (server.auth === "oauth") {
          auth = server.oauth_tokens ? t.mcpPanel.oauthTokens : t.mcpPanel.oauthPending;
        }
        value.textContent = `${server.kind} · ${server.target} · ${auth}`;
        value.title = server.target;
        row.append(label, value);
        if (server.auth === "oauth" && !server.oauth_tokens) {
          const connect = document.createElement("button");
          connect.className = "ghost mcp-connect";
          connect.textContent = t.mcpPanel.connect;
          connect.addEventListener("click", () => {
            this.startMcpOAuth(server, row, connect).catch(() => undefined);
          });
          row.appendChild(connect);
        }
        rows.appendChild(row);
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Initiate an MCP OAuth flow, surface the authorization URL, and
   * poll the flow until approved/error (hermes dashboard parity). */
  private async startMcpOAuth(
    server: McpServerRow,
    row: HTMLElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const client = this.client();
    if (!client) return;
    button.disabled = true;
    button.textContent = t.mcpPanel.connecting;
    let flow: McpOAuthFlow;
    try {
      flow = await client.mcpAuth(server.name);
    } catch (error) {
      button.disabled = false;
      button.textContent = t.mcpPanel.connect;
      this.mcpFlowNote(row, error instanceof Error ? error.message : String(error), true);
      return;
    }
    const flowId = flow.flow_id;
    const poll = window.setInterval(async () => {
      const current = this.client();
      if (!current) return;
      try {
        flow = await current.mcpFlowStatus(flowId);
      } catch (error) {
        window.clearInterval(poll);
        this.mcpPollers = this.mcpPollers.filter((id) => id !== poll);
        button.disabled = false;
        button.textContent = t.mcpPanel.connect;
        this.mcpFlowNote(row, error instanceof Error ? error.message : String(error), true);
        return;
      }
      if (flow.authorization_url && !row.querySelector(".mcp-auth-link")) {
        this.mcpAuthLink(row, flow.authorization_url);
      }
      if (flow.status === "approved") {
        window.clearInterval(poll);
        this.mcpPollers = this.mcpPollers.filter((id) => id !== poll);
        this.mcpFlowNote(row, t.mcpPanel.approved, false);
        window.setTimeout(() => this.loadMcp().catch(() => undefined), 500);
      } else if (flow.status === "error") {
        window.clearInterval(poll);
        this.mcpPollers = this.mcpPollers.filter((id) => id !== poll);
        button.disabled = false;
        button.textContent = t.mcpPanel.connect;
        this.mcpFlowNote(row, flow.error || t.mcpPanel.failed, true);
      }
    }, 2_000);
    this.mcpPollers.push(poll);
  }

  private mcpAuthLink(row: HTMLElement, url: string): void {
    const link = document.createElement("a");
    link.className = "mcp-auth-link";
    link.href = url;
    link.target = "_blank";
    link.rel = "noreferrer";
    link.textContent = t.mcpPanel.openAuth;
    row.appendChild(link);
  }

  private mcpFlowNote(row: HTMLElement, message: string, isError: boolean): void {
    let note = row.querySelector(".mcp-flow-note") as HTMLElement | null;
    if (!note) {
      note = document.createElement("span");
      note.className = "mcp-flow-note config-note";
      row.appendChild(note);
    }
    note.textContent = message;
    note.classList.toggle("error", isError);
  }

  /** Kanban diagnostics: boards with open/total counts, current-board
   * status histogram, and the blocked-task list. */
  private async loadKanban(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-kanban") as HTMLElement;
    const rows = this.root.querySelector("#kanban-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const boards = await client.kanbanBoards();
      rows.innerHTML = "";
      if (boards.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = t.kanbanPanel.none;
        rows.appendChild(empty);
        section.hidden = false;
        return;
      }
      for (const board of boards) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = `${board.name} (${board.slug})`;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        value.textContent =
          t.kanbanPanel.openOf
            .replace("{open}", String(board.open_tasks))
            .replace("{total}", String(board.total_tasks)) +
          (board.current ? ` · ${t.kanbanPanel.current}` : "");
        row.append(label, value);
        rows.appendChild(row);
      }
      const current = boards.find((board) => board.current);
      if (current) {
        const tasks = await client.kanbanTasks(current.slug);
        const counts = new Map<string, number>();
        for (const task of tasks) {
          counts.set(task.status, (counts.get(task.status) || 0) + 1);
        }
        if (counts.size > 0) {
          const row = document.createElement("div");
          row.className = "monitoring-row";
          const label = document.createElement("span");
          label.className = "monitoring-label";
          label.textContent = t.kanbanPanel.byStatus;
          const value = document.createElement("span");
          value.className = "monitoring-value";
          value.textContent = Array.from(counts.entries())
            .map(([status, count]) => `${status}: ${count}`)
            .join(" · ");
          row.append(label, value);
          rows.appendChild(row);
        }
        const blocked = tasks.filter((task) => task.status === "blocked");
        if (blocked.length > 0) {
          const row = document.createElement("div");
          row.className = "monitoring-row";
          const label = document.createElement("span");
          label.className = "monitoring-label";
          label.textContent = t.kanbanPanel.blocked;
          const value = document.createElement("span");
          value.className = "monitoring-value";
          value.textContent = blocked
            .slice(0, 8)
            .map((task) => task.title || task.id.slice(0, 8))
            .join("; ");
          row.append(label, value);
          rows.appendChild(row);
        }
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadLogs(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-logs") as HTMLElement;
    const body = this.root.querySelector("#logs-body") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const level = (this.root.querySelector("#logs-level") as HTMLSelectElement).value || undefined;
      const payload = await client.logsTail(LOGS_LINES, level);
      body.textContent = payload.lines.join("\n");
      (this.root.querySelector("#logs-path") as HTMLElement).textContent = payload.path;
      section.hidden = false;
      const refresh = this.root.querySelector("#logs-refresh") as HTMLButtonElement;
      if (!refresh.dataset.wired) {
        refresh.dataset.wired = "1";
        refresh.addEventListener("click", () => this.loadLogs().catch(() => undefined));
        (this.root.querySelector("#logs-level") as HTMLSelectElement).addEventListener(
          "change",
          () => this.loadLogs().catch(() => undefined),
        );
      }
    } catch {
      section.hidden = true;
    }
  }

  private async loadBrowser(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-browser") as HTMLElement;
    const rows = this.root.querySelector("#browser-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const status = await client.browserStatus();
      rows.innerHTML = "";
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const entries: [string, string][] = [
        [t.browserPanel.configured, status.configured ? on : off],
      ];
      if (status.configured) {
        if (status.backend) entries.push([t.browserPanel.backend, status.backend]);
        if (status.mode) entries.push([t.browserPanel.mode, status.mode]);
        if (status.source) entries.push([t.browserPanel.source, status.source]);
        if (status.endpoint) entries.push([t.browserPanel.endpoint, status.endpoint]);
        if (status.backend === "camofox") {
          entries.push([t.browserPanel.available, status.available ? on : off]);
          if (status.vnc_url) entries.push([t.browserPanel.vnc, status.vnc_url]);
        } else if (status.mode === "managed") {
          entries.push([t.browserPanel.managedRunning, status.managed_running ? on : off]);
        }
      }
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private render(sections: { title: string; checks: DoctorCheck[] }[], issues: string[]): void {
    const body = this.root.querySelector("#doctor-body") as HTMLElement;
    body.innerHTML = "";

    const issuesBox = document.createElement("div");
    issuesBox.className = issues.length > 0 ? "doctor-issues" : "doctor-clean";
    if (issues.length > 0) {
      const title = document.createElement("h3");
      title.textContent = `${t.doctor.issues} (${issues.length})`;
      issuesBox.appendChild(title);
      const list = document.createElement("ul");
      for (const issue of issues) {
        const item = document.createElement("li");
        item.textContent = issue;
        list.appendChild(item);
      }
      issuesBox.appendChild(list);
    } else {
      issuesBox.textContent = t.doctor.noIssues;
    }
    body.appendChild(issuesBox);

    if (sections.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.doctor.empty;
      body.appendChild(empty);
      return;
    }
    for (const section of sections) {
      const header = document.createElement("h3");
      header.className = "config-section";
      header.textContent = section.title;
      body.appendChild(header);
      for (const check of section.checks) {
        const row = document.createElement("div");
        row.className = `doctor-row doctor-${check.level}`;
        const icon = document.createElement("span");
        icon.className = "doctor-icon";
        icon.textContent = LEVEL_ICON[check.level] ?? "?";
        const text = document.createElement("span");
        text.className = "doctor-text";
        text.textContent = check.text;
        row.appendChild(icon);
        row.appendChild(text);
        if (check.detail) {
          const detail = document.createElement("span");
          detail.className = "doctor-detail";
          detail.textContent = check.detail;
          row.appendChild(detail);
        }
        body.appendChild(row);
      }
    }
  }
}
