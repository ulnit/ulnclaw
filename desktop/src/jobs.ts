// Cron jobs widget — dashboard over the gateway `/api/jobs` API (same
// cron store as the `ulnclaw cron` CLI): list/pause/resume/run-now,
// create + edit + delete. Hermes cron-dashboard parity (P166).

import type { CronJob, GatewayClient } from "./gateway";

function fmtWhen(ts: number | null): string {
  if (!ts) return "—";
  const date = new Date(ts * 1000);
  const delta = date.getTime() / 1000 - Date.now() / 1000;
  const abs = Math.abs(delta);
  const suffix = delta >= 0 ? "from now" : "ago";
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
        <button id="jobs-refresh" class="ghost" title="Refresh">↻</button>
        <button id="jobs-new" class="primary">New job</button>
      </header>
      <div id="jobs-list"></div>
      <dialog id="job-create">
        <form method="dialog">
          <h2>New cron job</h2>
          <label>Name
            <input id="job-create-name" type="text" placeholder="daily digest" required />
          </label>
          <label>Schedule (cron expression, @every 30m, or @at unix-ts)
            <input id="job-create-schedule" type="text" placeholder="0 9 * * *" required />
          </label>
          <label>Prompt
            <textarea id="job-create-prompt" rows="3" placeholder="What should the agent do?" required></textarea>
          </label>
          <label>Skills (comma-separated, optional)
            <input id="job-create-skills" type="text" placeholder="blogwatcher" />
          </label>
          <label>Repeat (runs remaining; empty = forever)
            <input id="job-create-repeat" type="number" min="1" placeholder="forever" />
          </label>
          <menu>
            <button value="cancel">Cancel</button>
            <button id="job-create-save" value="save">Create</button>
          </menu>
        </form>
      </dialog>`;

    (this.root.querySelector("#jobs-refresh") as HTMLButtonElement).onclick = () =>
      void this.refresh();
    (this.root.querySelector("#jobs-new") as HTMLButtonElement).onclick = () => {
      const dialog = this.root.querySelector("#job-create") as HTMLDialogElement;
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
    const created = await client.jobCreate({
      name,
      schedule,
      prompt,
      skills: skills.length ? skills : undefined,
      repeat: repeat && repeat > 0 ? repeat : undefined,
    });
    if (!created) {
      window.alert("Job creation failed (gateway unreachable or invalid schedule).");
      return;
    }
    (this.root.querySelector("#job-create-name") as HTMLInputElement).value = "";
    (this.root.querySelector("#job-create-schedule") as HTMLInputElement).value = "";
    (this.root.querySelector("#job-create-prompt") as HTMLTextAreaElement).value = "";
    (this.root.querySelector("#job-create-skills") as HTMLInputElement).value = "";
    (this.root.querySelector("#job-create-repeat") as HTMLInputElement).value = "";
    await this.refresh();
  }

  private render(): void {
    const counts = this.root.querySelector("#jobs-counts") as HTMLElement;
    const enabled = this.jobs.filter((job) => job.enabled).length;
    counts.textContent = `${enabled} active / ${this.jobs.length} jobs`;

    const list = this.root.querySelector("#jobs-list") as HTMLElement;
    list.innerHTML = "";
    if (!this.jobs.length) {
      list.innerHTML =
        '<p class="jobs-empty">No cron jobs yet — create one, or use `ulnclaw cron add` in the terminal.</p>';
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
    dot.title = job.enabled ? "Active" : "Paused";
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
    body.appendChild(nameLine);
    const prompt = document.createElement("div");
    prompt.className = "job-prompt";
    prompt.textContent = job.prompt;
    body.appendChild(prompt);
    const meta = document.createElement("div");
    meta.className = "job-meta";
    meta.textContent = `next: ${fmtWhen(job.next_run)} · last: ${fmtWhen(job.last_run)}` +
      (job.last_status ? ` (${job.last_status})` : "") +
      (job.repeat !== null ? ` · ${job.repeat} run(s) left` : "");
    body.appendChild(meta);
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
    mk(job.enabled ? "⏸" : "▶", job.enabled ? "Pause" : "Resume", () => {
      if (!client) return;
      void client.jobSetEnabled(job.id, !job.enabled).then(() => void this.refresh());
    });
    mk("⚡", "Run now", () => {
      if (!client) return;
      void client.jobRunNow(job.id).then(() => void this.refresh());
    });
    mk("✎", "Edit prompt/schedule", () => {
      if (!client) return;
      const nextPrompt = window.prompt("Prompt:", job.prompt);
      if (nextPrompt === null) return;
      const nextSchedule = window.prompt("Schedule:", job.schedule);
      if (nextSchedule === null) return;
      void client
        .jobUpdate(job.id, { prompt: nextPrompt, schedule: nextSchedule })
        .then(() => void this.refresh());
    });
    mk("🗑", "Delete", () => {
      if (!client) return;
      if (!window.confirm(`Delete job "${job.name}"?`)) return;
      void client.jobDelete(job.id).then(() => void this.refresh());
    }, true);
    row.appendChild(actions);
    return row;
  }
}
