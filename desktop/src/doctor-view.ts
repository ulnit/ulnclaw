// Doctor view — runs the gateway doctor checks over `GET /api/doctor`
// (the same report `ulnclaw doctor` prints) and renders ✓/⚠/✗/ℹ rows
// grouped by section, with an issues panel up top. Online provider
// probes are opt-in since they are slow.

import type { GatewayClient, DoctorCheck } from "./gateway";
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
    `;
    this.root.querySelector("#doctor-run")!.addEventListener("click", () => {
      this.run().catch(() => undefined);
    });
  }

  start(): void {
    if (!this.root.querySelector(".doctor-section")) {
      this.run().catch(() => undefined);
    }
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
