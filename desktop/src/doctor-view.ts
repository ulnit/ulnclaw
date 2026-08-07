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

function escapeHtmlDoctor(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

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
      <section id="doctor-system" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="systemPanel.title">System</h3>
        <div id="system-rows"></div>
      </section>
      <section id="doctor-storage" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="storagePanel.title">Session store</h3>
        <div id="storage-rows"></div>
      </section>
      <section id="doctor-backups" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="backupsPanel.title">State snapshots</h3>
        <div id="backups-rows"></div>
      </section>
      <section id="doctor-checkpoints" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="checkpointsPanel.title">Checkpoints</h3>
        <div id="checkpoints-rows"></div>
      </section>
      <section id="doctor-kanban" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="kanbanPanel.title">Kanban diagnostics</h3>
        <div id="kanban-rows"></div>
      </section>
      <section id="doctor-channels" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="channelsPanel.title">Messaging channels</h3>
        <div id="channels-rows"></div>
      </section>
      <section id="doctor-egress" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="egressPanel.title">Egress proxy</h3>
        <pre id="egress-body" class="logs-body"></pre>
      </section>
      <section id="doctor-learning" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="learningPanel.title">Learning graph</h3>
        <div id="learning-rows"></div>
      </section>
      <section id="doctor-update" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="updatePanel.title">Update</h3>
        <div class="logs-controls">
          <button id="update-check-btn" class="ghost" data-i18n="updatePanel.check">Check for updates</button>
          <button id="update-apply-btn" class="ghost danger" data-i18n="updatePanel.apply" hidden>Apply update</button>
          <span id="update-status" class="config-note"></span>
        </div>
        <pre id="update-body" class="logs-body" hidden></pre>
      </section>
      <section id="doctor-ops" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="opsPanel.title">Ops actions</h3>
        <div class="logs-controls">
          <button id="ops-audit-btn" class="ghost" data-i18n="opsPanel.securityAudit">Security audit</button>
          <button id="ops-prompt-size-btn" class="ghost" data-i18n="opsPanel.promptSize">Prompt size</button>
          <button id="ops-dump-btn" class="ghost" data-i18n="opsPanel.dump">Debug dump</button>
          <span id="ops-status" class="config-note"></span>
        </div>
        <pre id="ops-body" class="logs-body" hidden></pre>
      </section>
      <section id="doctor-metrics" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="metricsPanel.title">Prometheus metrics</h3>
        <details id="metrics-details">
          <summary data-i18n="metricsPanel.summary">Show raw /metrics exposition</summary>
          <pre id="metrics-body" class="logs-body"></pre>
        </details>
      </section>
      <section id="doctor-logs" class="doctor-monitoring doctor-logs" hidden>
        <h3 class="config-section" data-i18n="logsPanel.title">Gateway log</h3>
        <div class="logs-controls">
          <select id="logs-file">
            <option value="gateway">gateway.log</option>
            <option value="agent">agent.log</option>
            <option value="errors">errors.log</option>
          </select>
          <input id="logs-search" type="text" data-i18n-ph="logsPanel.searchPlaceholder" placeholder="search\u2026" />
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
    this.root.querySelector("#ops-audit-btn")!.addEventListener("click", () => {
      this.runOps("securityAudit").catch(() => undefined);
    });
    this.root.querySelector("#ops-prompt-size-btn")!.addEventListener("click", () => {
      this.runOps("promptSize").catch(() => undefined);
    });
    this.root.querySelector("#ops-dump-btn")!.addEventListener("click", () => {
      this.runOps("dump").catch(() => undefined);
    });
    this.root.querySelector("#update-check-btn")!.addEventListener("click", () => {
      this.checkUpdate().catch(() => undefined);
    });
    this.root.querySelector("#update-apply-btn")!.addEventListener("click", () => {
      this.applyUpdate().catch(() => undefined);
    });
  }

  start(): void {
    if (!this.root.querySelector(".doctor-section")) {
      this.run().catch(() => undefined);
    }
    this.loadMonitoring().catch(() => undefined);
    this.loadBrowser().catch(() => undefined);
    this.loadMcp().catch(() => undefined);
    this.loadSystem().catch(() => undefined);
    this.loadStorage().catch(() => undefined);
    this.loadBackups().catch(() => undefined);
    this.loadCheckpoints().catch(() => undefined);
    this.loadKanban().catch(() => undefined);
    this.loadMetrics().catch(() => undefined);
    this.loadEgress().catch(() => undefined);
    this.loadChannels().catch(() => undefined);
    this.loadLearning().catch(() => undefined);
    this.loadOps().catch(() => undefined);
    this.loadUpdate().catch(() => undefined);
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
        const tools = server.cached_tools || [];
        if (tools.length > 0) {
          const details = document.createElement("details");
          details.className = "mcp-tools-details";
          details.innerHTML = `<summary>${
            t.mcpPanel.toolsCached.replace("{count}", String(tools.length))
          }</summary>`;
          const list = document.createElement("div");
          list.className = "mcp-tools-list";
          for (const tool of tools) {
            const item = document.createElement("div");
            item.className = "mcp-tool";
            const name = document.createElement("code");
            name.textContent = tool.name;
            item.appendChild(name);
            if (tool.description) {
              const desc = document.createElement("span");
              desc.className = "mcp-tool-desc";
              desc.textContent = tool.description;
              item.appendChild(desc);
            }
            list.appendChild(item);
          }
          details.appendChild(list);
          row.appendChild(details);
        }
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

  private fmtUptime(seconds: number): string {
    const days = Math.floor(seconds / 86_400);
    const hours = Math.floor((seconds % 86_400) / 3_600);
    const minutes = Math.floor((seconds % 3_600) / 60);
    if (days > 0) return `${days}d ${hours}h`;
    if (hours > 0) return `${hours}h ${minutes}m`;
    return `${minutes}m`;
  }

  /** Gateway/system facts: version, platform, paths, uptime, counts. */
  private async loadSystem(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-system") as HTMLElement;
    const rows = this.root.querySelector("#system-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const info = await client.systemInfo();
      const v = t.systemPanel;
      const entries: [string, string][] = [
        [v.version, `${info.service} ${info.version}`],
        [v.platform, `${info.os}/${info.arch} · pid ${info.pid}${info.desktop_managed ? ` · ${v.desktopManaged}` : ""}`],
        [v.uptime, this.fmtUptime(info.uptime_secs)],
        [v.contents, `${info.sessions} ${v.sessionsWord} · ${info.messages} ${v.messagesWord} · ${info.active_runs} ${v.runsWord}`],
        [v.jobs, `${info.cron_jobs_enabled} ${v.enabledWord} · ${info.cron_jobs_disabled} ${v.disabledWord}`],
        [v.plugins, String(info.plugins_loaded)],
        [v.home, info.home],
        [v.config, info.config_path],
      ];
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** State snapshots (P315): quick-backup inventory with
   * create/restore/prune over /api/backups (hermes `backup` parity). */
  private async loadBackups(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-backups") as HTMLElement;
    const rows = this.root.querySelector("#backups-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const snapshots = await client.backupsList();
      rows.innerHTML = "";
      if (!snapshots.length) {
        const empty = document.createElement("div");
        empty.className = "monitoring-row config-note";
        empty.textContent = t.backupsPanel.empty;
        rows.appendChild(empty);
      }
      for (const snapshot of snapshots) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = snapshot.id;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        value.textContent = `${snapshot.files} files \u00b7 ${this.fmtBytes(snapshot.bytes)}`;
        const restore = document.createElement("button");
        restore.className = "ghost";
        restore.textContent = t.backupsPanel.restore;
        restore.onclick = () => this.restoreBackup(snapshot.id);
        row.append(label, value, restore);
        rows.appendChild(row);
      }
      const actions = document.createElement("div");
      actions.className = "monitoring-row";
      const create = document.createElement("button");
      create.className = "ghost";
      create.textContent = t.backupsPanel.newSnapshot;
      create.onclick = () => this.createBackup();
      const prune = document.createElement("button");
      prune.className = "ghost";
      prune.textContent = t.backupsPanel.prune;
      prune.onclick = () => this.pruneBackups();
      const status = document.createElement("span");
      status.className = "config-note";
      status.id = "backups-status";
      actions.append(create, prune, status);
      rows.appendChild(actions);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Checkpoint store census (P317): sizes + per-project rows with a
   * prune action, over /api/checkpoints (hermes `checkpoint` parity). */
  private async loadCheckpoints(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-checkpoints") as HTMLElement;
    const rows = this.root.querySelector("#checkpoints-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const status = await client.checkpointsStatus();
      rows.innerHTML = "";
      const sizeRow = document.createElement("div");
      sizeRow.className = "monitoring-row";
      const sizeLabel = document.createElement("span");
      sizeLabel.className = "monitoring-label";
      sizeLabel.textContent = t.checkpointsPanel.size;
      const sizeValue = document.createElement("span");
      sizeValue.className = "monitoring-value";
      sizeValue.textContent = `${this.fmtBytes(status.store_size_bytes)} / ${this.fmtBytes(status.total_size_bytes)}`;
      sizeRow.append(sizeLabel, sizeValue);
      rows.appendChild(sizeRow);

      if (!status.projects.length) {
        const empty = document.createElement("div");
        empty.className = "monitoring-row config-note";
        empty.textContent = t.checkpointsPanel.noProjects;
        rows.appendChild(empty);
      }
      for (const project of status.projects) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = project.workdir || project.hash.slice(0, 12);
        label.title = project.workdir;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        value.textContent = `${project.commits} commits${project.exists ? "" : " \u00b7 missing dir"}`;
        row.append(label, value);
        rows.appendChild(row);
      }

      const actions = document.createElement("div");
      actions.className = "monitoring-row";
      const prune = document.createElement("button");
      prune.className = "ghost";
      prune.textContent = t.checkpointsPanel.prune;
      prune.onclick = () => this.pruneCheckpoints();
      const statusEl = document.createElement("span");
      statusEl.className = "config-note";
      statusEl.id = "checkpoints-status-note";
      actions.append(prune, statusEl);
      rows.appendChild(actions);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async pruneCheckpoints(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const raw = window.prompt(t.checkpointsPanel.prunePrompt, "7");
    if (raw === null) return;
    const days = Number.parseInt(raw, 10);
    if (!Number.isFinite(days) || days < 0) return;
    const note = this.root.querySelector("#checkpoints-status-note");
    try {
      const stats = await client.checkpointsPrune(days);
      await this.loadCheckpoints();
      const el = this.root.querySelector("#checkpoints-status-note");
      if (el) {
        el.textContent = t.checkpointsPanel.pruned
          .replace("{orphan}", String(stats.deleted_orphan))
          .replace("{stale}", String(stats.deleted_stale))
          .replace("{bytes}", this.fmtBytes(stats.bytes_freed));
      }
    } catch (error) {
      if (note) {
        note.textContent = t.checkpointsPanel.pruneFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        );
      }
    }
  }

  private backupStatus(message: string): void {
    const el = this.root.querySelector("#backups-status");
    if (el) el.textContent = message;
  }

  private async createBackup(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const label = window.prompt(t.backupsPanel.labelPrompt, "");
    if (label === null) return;
    try {
      const result = await client.backupCreate(label.trim() || undefined);
      await this.loadBackups();
      this.backupStatus(
        result.id
          ? t.backupsPanel.created.replace("{id}", result.id)
          : result.message || "",
      );
    } catch (error) {
      this.backupStatus(
        t.backupsPanel.createFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private async restoreBackup(id: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.backupsPanel.restoreConfirm.replace("{id}", id))) return;
    try {
      await client.backupRestore(id);
      await this.loadBackups();
      this.backupStatus(t.backupsPanel.restored.replace("{id}", id));
    } catch (error) {
      this.backupStatus(
        t.backupsPanel.restoreFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private async pruneBackups(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const raw = window.prompt(t.backupsPanel.prunePrompt, "20");
    if (raw === null) return;
    const keep = Number.parseInt(raw, 10);
    if (!Number.isFinite(keep) || keep < 1) return;
    try {
      const removed = await client.backupPrune(keep);
      await this.loadBackups();
      this.backupStatus(t.backupsPanel.pruned.replace("{count}", String(removed)));
    } catch (error) {
      this.backupStatus(
        t.backupsPanel.pruneFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private fmtBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  /** Session-store footprint + one-click optimize (FTS merge + VACUUM,
   * same work as `ulnclaw sessions optimize`). */
  private async loadStorage(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-storage") as HTMLElement;
    const rows = this.root.querySelector("#storage-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const stats = await client.storageStats();
      rows.innerHTML = "";
      const sizeRow = document.createElement("div");
      sizeRow.className = "monitoring-row";
      const sizeLabel = document.createElement("span");
      sizeLabel.className = "monitoring-label";
      sizeLabel.textContent = t.storagePanel.size;
      const sizeValue = document.createElement("span");
      sizeValue.className = "monitoring-value";
      sizeValue.textContent = `${this.fmtBytes(stats.size_bytes)}${
        stats.wal_bytes > 0 ? ` + ${this.fmtBytes(stats.wal_bytes)} WAL` : ""
      }`;
      sizeRow.append(sizeLabel, sizeValue);
      rows.appendChild(sizeRow);

      const countsRow = document.createElement("div");
      countsRow.className = "monitoring-row";
      const countsLabel = document.createElement("span");
      countsLabel.className = "monitoring-label";
      countsLabel.textContent = t.storagePanel.contents;
      const countsValue = document.createElement("span");
      countsValue.className = "monitoring-value";
      countsValue.textContent = t.storagePanel.counts
        .replace("{sessions}", String(stats.sessions))
        .replace("{messages}", String(stats.messages));
      countsRow.append(countsLabel, countsValue);
      rows.appendChild(countsRow);

      const pathRow = document.createElement("div");
      pathRow.className = "monitoring-row";
      const pathLabel = document.createElement("span");
      pathLabel.className = "monitoring-label";
      pathLabel.textContent = t.storagePanel.path;
      const pathValue = document.createElement("span");
      pathValue.className = "monitoring-value";
      pathValue.textContent = stats.db_path;
      pathValue.title = stats.db_path;
      pathRow.append(pathLabel, pathValue);
      rows.appendChild(pathRow);

      const actionRow = document.createElement("div");
      actionRow.className = "monitoring-row";
      const spacer = document.createElement("span");
      spacer.className = "monitoring-label";
      const optimize = document.createElement("button");
      optimize.className = "ghost";
      optimize.textContent = t.storagePanel.optimize;
      optimize.title = t.storagePanel.optimizeTitle;
      const note = document.createElement("span");
      note.className = "config-note storage-note";
      optimize.addEventListener("click", async () => {
        const current = this.client();
        if (!current) return;
        optimize.disabled = true;
        optimize.textContent = t.storagePanel.optimizing;
        try {
          const result = await current.storageOptimize();
          note.textContent = t.storagePanel.optimized
            .replace("{indexes}", String(result.merged_indexes))
            .replace("{before}", this.fmtBytes(result.before_bytes))
            .replace("{after}", this.fmtBytes(result.after_bytes));
          this.loadStorage().catch(() => undefined);
        } catch (error) {
          note.textContent = t.storagePanel.optimizeFailed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          );
          note.classList.add("error");
          optimize.disabled = false;
          optimize.textContent = t.storagePanel.optimize;
        }
      });
      actionRow.append(spacer, optimize, note);
      rows.appendChild(actionRow);

      section.hidden = false;
    } catch {
      section.hidden = true;
    }
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

  /** Messaging-platform enabled posture (hermes ChannelsPage parity). */
  private async loadChannels(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-channels") as HTMLElement;
    const rows = this.root.querySelector("#channels-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const channels = await client.channels();
      const enabled = channels.filter((channel) => channel.enabled);
      const disabled = channels.filter((channel) => !channel.enabled);
      rows.innerHTML = "";

      const enabledRow = document.createElement("div");
      enabledRow.className = "monitoring-row";
      const enabledLabel = document.createElement("span");
      enabledLabel.className = "monitoring-label";
      enabledLabel.textContent = t.channelsPanel.enabled;
      const enabledValue = document.createElement("span");
      enabledValue.className = "monitoring-value";
      enabledValue.innerHTML = enabled.length
        ? enabled
            .map((channel) => `<span class="models-view-badge ok">${escapeHtmlDoctor(channel.name)}</span>`)
            .join(" ")
        : escapeHtmlDoctor(t.channelsPanel.noneEnabled);
      enabledRow.append(enabledLabel, enabledValue);
      rows.appendChild(enabledRow);

      const disabledRow = document.createElement("div");
      disabledRow.className = "monitoring-row";
      const disabledLabel = document.createElement("span");
      disabledLabel.className = "monitoring-label";
      disabledLabel.textContent = t.channelsPanel.disabled;
      const disabledValue = document.createElement("span");
      disabledValue.className = "monitoring-value channels-disabled";
      disabledValue.textContent = disabled.map((channel) => channel.name).join(", ");
      disabledRow.append(disabledLabel, disabledValue);
      rows.appendChild(disabledRow);

      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Learning-graph census (hermes star-map parity): node/edge counts,
   * density, and top clusters from GET /api/learning/graph. */
  private async loadLearning(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-learning") as HTMLElement;
    const rows = this.root.querySelector("#learning-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const graph = await client.learningGraph();
      const num = (key: string): number => {
        const value = graph.stats[key];
        return typeof value === "number" ? value : 0;
      };
      const skillNodes = graph.nodes.filter((node) => node.kind === "skill").length;
      const memoryNodes = graph.nodes.filter((node) => node.kind === "memory").length;
      const v = t.learningPanel;
      const entries: [string, string][] = [
        [v.skills, String(num("learned_skills") || skillNodes)],
        [v.memoryNodes, String(num("memory_nodes") || memoryNodes)],
        [
          v.edges,
          `${num("related_edges") + num("memory_skill_edges")} (${num("related_edges")} ${v.skillEdgesWord} \u00b7 ${num("memory_skill_edges")} ${v.memoryEdgesWord})`,
        ],
        [v.density, String(num("edges_per_node"))],
        [v.linked, `${num("linked_nodes")} (${num("isolated_pct")}% ${v.isolated})`],
        [v.origin, `${num("agent_created")} ${v.agentCreatedWord} \u00b7 ${num("used")} ${v.usedWord}`],
        [v.categories, String(num("categories") || graph.clusters.length)],
      ];
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      if (graph.clusters.length > 0) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = v.topCategories;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = graph.clusters
          .slice(0, 8)
          .map((cluster) => `${cluster.category} \u00d7${cluster.count}`)
          .join(" \u00b7 ");
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      const hint = document.createElement("div");
      hint.className = "config-note";
      hint.textContent = v.hint;
      rows.appendChild(hint);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Egress-proxy status text (tokens redacted server-side). */
  private async loadEgress(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-egress") as HTMLElement;
    const body = this.root.querySelector("#egress-body") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      body.textContent = await client.egressStatus();
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Show the update panel when a gateway is connected (P324). */
  private async loadUpdate(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-update") as HTMLElement;
    section.hidden = !client;
  }

  /** GET /api/update/check and render the outcome (P324). */
  private async checkUpdate(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const note = this.root.querySelector("#update-status") as HTMLElement;
    const body = this.root.querySelector("#update-body") as HTMLPreElement;
    const applyBtn = this.root.querySelector("#update-apply-btn") as HTMLButtonElement;
    note.textContent = t.updatePanel.checking;
    body.hidden = true;
    applyBtn.hidden = true;
    try {
      const result = await client.updateCheck();
      let headline: string;
      if (result.error) {
        headline = t.updatePanel.checkFailed.replace("{error}", result.error);
      } else if (result.behind === 0) {
        headline = `${t.updatePanel.upToDate} (${result.current_version})`;
      } else if (result.behind === -1) {
        headline = t.updatePanel.behindShallow;
      } else {
        headline = t.updatePanel.behind
          .replace("{count}", String(result.behind ?? 0))
          .replace("{version}", result.current_version);
      }
      note.textContent = headline;
      if (result.log && result.log.length) {
        body.textContent = result.log.join("\n");
        body.hidden = false;
      }
      applyBtn.hidden = !(result.update_available && result.can_apply);
    } catch (error) {
      note.textContent = t.updatePanel.checkFailed.replace("{error}", String(error));
    }
  }

  /** POST /api/update after confirmation and render the report (P324). */
  private async applyUpdate(): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.updatePanel.applyConfirm)) return;
    const note = this.root.querySelector("#update-status") as HTMLElement;
    const body = this.root.querySelector("#update-body") as HTMLPreElement;
    note.textContent = t.updatePanel.applying;
    try {
      const report = await client.updateApply();
      note.textContent = t.updatePanel.applyDone
        .replace("{commits}", String(report.new_commits))
        .replace("{sha}", (report.new_sha || "").slice(0, 8));
      body.textContent = report.log.join("\n");
      body.hidden = false;
    } catch (error) {
      note.textContent = t.updatePanel.applyFailed.replace("{error}", String(error));
    }
  }

  /** Show the ops panel when a gateway is connected (P321). */
  private async loadOps(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-ops") as HTMLElement;
    section.hidden = !client;
  }

  /** Run an ops action over /api/ops/* and render the output (P321). */
  private async runOps(action: "securityAudit" | "promptSize" | "dump"): Promise<void> {
    const client = this.client();
    if (!client) return;
    const body = this.root.querySelector("#ops-body") as HTMLPreElement;
    const note = this.root.querySelector("#ops-status") as HTMLElement;
    const label = t.opsPanel[action];
    note.textContent = t.opsPanel.running.replace("{action}", label);
    body.hidden = true;
    try {
      let text: string;
      if (action === "securityAudit") {
        const report = await client.opsSecurityAudit();
        text =
          report.finding_count === 0
            ? report.note ?? t.opsPanel.auditClean.replace("{total}", String(report.total_components_scanned))
            : report.findings
                .map((finding) => {
                  const fix = finding.vuln.fixed_versions.length
                    ? ` (fix: ${finding.vuln.fixed_versions.join(", ")})`
                    : "";
                  return `[${finding.vuln.severity}] ${finding.component.name}@${finding.component.version} \u2014 ${finding.vuln.osv_id}: ${finding.vuln.summary}${fix}`;
                })
                .join("\n");
      } else if (action === "promptSize") {
        const report = await client.opsPromptSize();
        const lines = [
          `model: ${report.model} (${report.provider})`,
          `system prompt: ${report.system_prompt_chars} chars / ${report.system_prompt_bytes} bytes`,
          "",
          ...report.sections.map((row) => `  ${row.label}: ${row.chars} chars / ${row.bytes} bytes`),
          "",
          `tools: ${report.tools_count} tools / ${report.tools_json_bytes} bytes of JSON schema`,
          ...report.toolsets.map((row) => `  ${row.toolset}: ${row.tools} tools / ${row.json_bytes} bytes`),
          "",
          `skills: ${report.skills.length} installed / ${report.skills_total_bytes} bytes on disk`,
        ];
        text = lines.join("\n");
      } else {
        text = await client.opsDump();
      }
      body.textContent = text;
      body.hidden = false;
      note.textContent = "";
    } catch (error) {
      note.textContent = t.opsPanel.failed
        .replace("{action}", label)
        .replace("{error}", String(error));
    }
  }

  /** Raw Prometheus exposition from GET /metrics (collapsible). */
  private async loadMetrics(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-metrics") as HTMLElement;
    const body = this.root.querySelector("#metrics-body") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      body.textContent = await client.metricsRaw();
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
      const file = (this.root.querySelector("#logs-file") as HTMLSelectElement).value || "gateway";
      const search =
        (this.root.querySelector("#logs-search") as HTMLInputElement).value.trim() || undefined;
      const payload = await client.logsFile(file, { lines: LOGS_LINES, level, search });
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
        (this.root.querySelector("#logs-file") as HTMLSelectElement).addEventListener(
          "change",
          () => this.loadLogs().catch(() => undefined),
        );
        (this.root.querySelector("#logs-search") as HTMLInputElement).addEventListener(
          "keydown",
          (event) => {
            if (event.key === "Enter") this.loadLogs().catch(() => undefined);
          },
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
