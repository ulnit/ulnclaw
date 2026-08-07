// Doctor view — runs the gateway doctor checks over `GET /api/doctor`
// (the same report `ulnclaw doctor` prints) and renders ✓/⚠/✗/ℹ rows
// grouped by section, with an issues panel up top. Online provider
// probes are opt-in since they are slow.

import type { GatewayClient, DoctorCheck, MonitoringPayload } from "./gateway";
import { t } from "./i18n";

const LEVEL_ICON: Record<DoctorCheck["level"], string> = {
  ok: "✓",
  warn: "⚠",
  fail: "✗",
  info: "ℹ",
};

export class DoctorWidget {
  private busy = false;

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
  }

  stop(): void {
    /* nothing polls */
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
