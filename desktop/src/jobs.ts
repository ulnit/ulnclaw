// Cron jobs widget — dashboard over the gateway `/api/jobs` API (same
// cron store as the `ulnclaw cron` CLI): list/pause/resume/run-now,
// create + edit + delete. Hermes cron-dashboard parity (P166).

import type { CronJob, GatewayClient } from "./gateway";
import { fmt, t } from "./i18n";

function fmtWhen(ts: number | null): string {
  if (!ts) return "—";
  const date = new Date(ts * 1000);
  const delta = date.getTime() / 1000 - Date.now() / 1000;
  const abs = Math.abs(delta);
  const suffix = delta >= 0 ? t.jobs.fromNow : t.jobs.ago;
  if (abs < 60) return `${Math.round(abs)}s ${suffix}`;
  if (abs < 3600) return `${Math.round(abs / 60)}m ${suffix}`;
  if (abs < 86400) return `${Math.round(abs / 3600)}h ${suffix}`;
  return date.toLocaleString();
}

export class JobsWidget {
  private jobs: CronJob[] = [];
  private timer: number | null = null;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  /** Build the static skeleton once; `refresh` fills it with data. */
  mount(): void {
    this.root.innerHTML = `
      <header id="jobs-header">
        <span id="jobs-counts" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="jobs-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
        <button id="jobs-new" class="primary" data-i18n="jobs.newJob">New job</button>
      </header>
      <div id="jobs-list"></div>
      <dialog id="job-create">
        <form method="dialog">
          <h2 data-i18n="jobs.newCronJob">New cron job</h2>
          <label><span data-i18n="jobs.nameLabel">Name</span>
            <input id="job-create-name" type="text" placeholder="daily digest" required />
          </label>
          <label><span data-i18n="jobs.scheduleLabel">Schedule (cron expression, @every 30m, or @at unix-ts)</span>
            <input id="job-create-schedule" type="text" placeholder="0 9 * * *" required />
          </label>
          <label><span data-i18n="jobs.promptLabel">Prompt</span>
            <textarea id="job-create-prompt" rows="3" placeholder="What should the agent do?" data-i18n-ph="jobs.whatShouldAgentDo" required></textarea>
          </label>
          <label><span data-i18n="jobs.skillsLabel">Skills (comma-separated, optional)</span>
            <input id="job-create-skills" type="text" placeholder="blogwatcher" />
          </label>
          <label><span data-i18n="jobs.repeatLabel">Repeat (runs remaining; empty = forever)</span>
            <input id="job-create-repeat" type="number" min="1" placeholder="forever" />
          </label>
          <label><span data-i18n="jobs.deliverLabel">Deliver result to</span>
            <select id="job-create-deliver"></select>
          </label>
          <menu>
            <button value="cancel" data-i18n="chrome.cancel">Cancel</button>
            <button id="job-create-save" value="save" data-i18n="jobs.create">Create</button>
          </menu>
        </form>
      </dialog>`;

    (this.root.querySelector("#jobs-refresh") as HTMLButtonElement).onclick = () =>
      void this.refresh();
    (this.root.querySelector("#jobs-new") as HTMLButtonElement).onclick = () => {
      const dialog = this.root.querySelector("#job-create") as HTMLDialogElement;
      this.populateDeliverTargets().catch(() => undefined);
      dialog.showModal();
    };
    const dialog = this.root.querySelector("#job-create") as HTMLDialogElement;
    dialog.addEventListener("close", () => {
      if (dialog.returnValue !== "save") return;
      void this.createJob();
    });
  }

  /** Load data when the tab becomes visible; poll every 10 s. */
  start(): void {
    void this.refresh();
    if (this.timer === null) {
      this.timer = window.setInterval(() => void this.refresh(), 10000);
    }
  }

  stop(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
  }

  private async refresh(): Promise<void> {
    const client = this.client();
    if (!client) return;
    this.jobs = await client.jobsList(true);
    this.render();
  }

  private async populateDeliverTargets(): Promise<void> {
    const client = this.client();
    const select = this.root.querySelector("#job-create-deliver") as HTMLSelectElement;
    if (!client) return;
    try {
      const targets = await client.jobDeliveryTargets();
      select.innerHTML = targets
        .map((target) => `<option value="${target.id}">${target.name}</option>`)
        .join("");
    } catch {
      select.innerHTML = `<option value="local">local</option>`;
    }
  }

  private async createJob(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const name = (this.root.querySelector("#job-create-name") as HTMLInputElement).value.trim();
    const schedule = (this.root.querySelector("#job-create-schedule") as HTMLInputElement).value.trim();
    const prompt = (this.root.querySelector("#job-create-prompt") as HTMLTextAreaElement).value.trim();
    const skillsRaw = (this.root.querySelector("#job-create-skills") as HTMLInputElement).value;
    const repeatRaw = (this.root.querySelector("#job-create-repeat") as HTMLInputElement).value.trim();
    if (!name || !schedule || !prompt) return;
    const skills = skillsRaw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
    const repeat = repeatRaw ? Number(repeatRaw) : undefined;
    const deliver = (this.root.querySelector("#job-create-deliver") as HTMLSelectElement).value;
    const created = await client.jobCreate({
      name,
      schedule,
      prompt,
      skills: skills.length ? skills : undefined,
      repeat: repeat && repeat > 0 ? repeat : undefined,
      deliver: deliver || undefined,
    });
    if (!created) {
      window.alert(t.jobs.createFailed);
      return;
    }
    (this.root.querySelector("#job-create-name") as HTMLInputElement).value = "";
    (this.root.querySelector("#job-create-schedule") as HTMLInputElement).value = "";
    (this.root.querySelector("#job-create-prompt") as HTMLTextAreaElement).value = "";
    (this.root.querySelector("#job-create-skills") as HTMLInputElement).value = "";
    (this.root.querySelector("#job-create-repeat") as HTMLInputElement).value = "";
    await this.refresh();
  }

  /** Re-render cached data after a locale switch (P251). */
  rerender(): void {
    if (this.jobs.length) this.render();
  }

  private render(): void {
    const counts = this.root.querySelector("#jobs-counts") as HTMLElement;
    const enabled = this.jobs.filter((job) => job.enabled).length;
    counts.textContent = fmt(t.jobs.counts, { active: enabled, total: this.jobs.length });

    const list = this.root.querySelector("#jobs-list") as HTMLElement;
    list.innerHTML = "";
    if (!this.jobs.length) {
      list.innerHTML = '<p class="jobs-empty"></p>';
      list.querySelector(".jobs-empty")!.textContent = t.jobs.empty;
      return;
    }
    for (const job of this.jobs) {
      list.appendChild(this.renderRow(job));
    }
  }

  private renderRow(job: CronJob): HTMLElement {
    const client = this.client();
    const row = document.createElement("div");
    row.className = "job-row" + (job.enabled ? "" : " paused");

    const dot = document.createElement("span");
    dot.className = "dot " + (job.enabled ? "up" : "down");
    dot.title = job.enabled ? t.jobs.active : t.jobs.paused;
    row.appendChild(dot);

    const body = document.createElement("div");
    body.className = "job-body";
    const nameLine = document.createElement("div");
    nameLine.className = "job-name-line";
    const name = document.createElement("span");
    name.className = "job-name";
    name.textContent = job.name;
    nameLine.appendChild(name);
    const schedule = document.createElement("span");
    schedule.className = "job-schedule";
    schedule.textContent = job.schedule;
    nameLine.appendChild(schedule);
    if (job.skills?.length) {
      const skills = document.createElement("span");
      skills.className = "job-skills";
      skills.textContent = job.skills.join(", ");
      nameLine.appendChild(skills);
    }
    if (job.deliver) {
      const deliver = document.createElement("span");
      deliver.className = "job-skills";
      deliver.title = t.jobs.deliverTitle;
      deliver.textContent = fmt(t.jobs.deliverBadge, { target: job.deliver });
      nameLine.appendChild(deliver);
    }
    body.appendChild(nameLine);
    const prompt = document.createElement("div");
    prompt.className = "job-prompt";
    prompt.textContent = job.prompt;
    body.appendChild(prompt);
    const meta = document.createElement("div");
    meta.className = "job-meta";
    meta.textContent = fmt(t.jobs.meta, { next: fmtWhen(job.next_run), last: fmtWhen(job.last_run) }) +
      (job.last_status ? ` (${job.last_status})` : "") +
      (job.repeat !== null ? fmt(t.jobs.runsLeft, { count: job.repeat }) : "");
    body.appendChild(meta);
    if (job.last_delivery_error) {
      const deliveryError = document.createElement("div");
      deliveryError.className = "job-meta job-delivery-error";
      deliveryError.title = job.last_delivery_error;
      deliveryError.textContent = `\u26a0 ${t.jobs.deliveryError}`;
      body.appendChild(deliveryError);
    }
    row.appendChild(body);

    const actions = document.createElement("div");
    actions.className = "job-actions";
    const mk = (label: string, title: string, onclick: () => void, danger = false): void => {
      const button = document.createElement("button");
      button.className = danger ? "ghost danger" : "ghost";
      button.textContent = label;
      button.title = title;
      button.onclick = onclick;
      actions.appendChild(button);
    };
    mk(job.enabled ? "⏸" : "▶", job.enabled ? t.jobs.pause : t.jobs.resume, () => {
      if (!client) return;
      void client.jobSetEnabled(job.id, !job.enabled).then(() => void this.refresh());
    });
    mk("⚡", t.jobs.runNow, () => {
      if (!client) return;
      void client.jobRunNow(job.id).then(() => void this.refresh());
    });
    mk("✎", t.jobs.edit, () => {
      if (!client) return;
      const nextPrompt = window.prompt(t.jobs.promptPrompt, job.prompt);
      if (nextPrompt === null) return;
      const nextSchedule = window.prompt(t.jobs.schedulePrompt, job.schedule);
      if (nextSchedule === null) return;
      const nextDeliver = window.prompt(t.jobs.deliverPrompt, job.deliver || "");
      if (nextDeliver === null) return;
      void client
        .jobUpdate(job.id, {
          prompt: nextPrompt,
          schedule: nextSchedule,
          deliver: nextDeliver.trim() || null,
        })
        .then(() => void this.refresh());
    });
    mk("🗑", t.jobs.delete, () => {
      if (!client) return;
      if (!window.confirm(fmt(t.jobs.deleteConfirm, { name: job.name }))) return;
      void client.jobDelete(job.id).then(() => void this.refresh());
    }, true);
    row.appendChild(actions);
    return row;
  }
}
