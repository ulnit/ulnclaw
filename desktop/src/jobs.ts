import { ICON } from "./icons";
// Cron jobs widget — dashboard over the gateway `/api/jobs` API (same
// cron store as the `ulnclaw cron` CLI): list/pause/resume/run-now,
// create + edit + delete. Hermes cron-dashboard parity (P166).

import type { CronJob, GatewayClient, JobBlueprint, JobSuggestion } from "./gateway";
import { fmt, t } from "./i18n";

const SORT_KEY = "ulnclaw.jobs.sort";

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
  /** Known delivery-target ids for P507 edit-time validation. */
  private deliveryTargetIds = new Set<string>();
  /** P536: live filter text over name/schedule/prompt/skills. */
  private filterText = "";
  /** P655: cached automation-blueprint catalog. */
  private blueprints: JobBlueprint[] = [];
  /** P655: blueprint currently open in the slot form. */
  private activeBlueprint: JobBlueprint | null = null;
  /** P673: pending automation suggestions. */
  private suggestions: JobSuggestion[] = [];

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
        <input id="jobs-filter" type="search" data-i18n-ph="jobs.filterPlaceholder" />
        <select id="jobs-sort" data-i18n-title="jobs.sortTitle">
          <option value="default" data-i18n="jobs.sortDefault">Default order</option>
          <option value="next_run" data-i18n="jobs.sortNextRun">Next run first</option>
        </select>
        <button id="jobs-refresh" class="ghost icon-btn" title="Refresh" data-i18n-title="kanban.refresh">${ICON.rotate}</button>
        <button id="jobs-blueprints" class="ghost" data-i18n="jobs.blueprintsAction">Templates</button>
        <button id="jobs-ideas" class="ghost" data-i18n="jobs.ideasAction">Ideas</button>
        <button id="jobs-new" class="primary" data-i18n="jobs.newJob">New job</button>
      </header>
      <section id="jobs-suggestions" hidden>
        <h3 class="config-section" data-i18n="jobs.suggestionsTitle">Suggested automations</h3>
        <div id="jobs-suggestions-list" class="jobs-suggestions-list"></div>
      </section>
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
      </dialog>
      <dialog id="jobs-blueprint-gallery">
        <form method="dialog">
          <h2 data-i18n="jobs.blueprintsTitle">Automation templates</h2>
          <input id="jobs-blueprint-filter" type="search" data-i18n-ph="jobs.blueprintsFilterPh" />
          <div id="jobs-blueprint-cards" class="jobs-blueprint-cards"></div>
          <div id="jobs-blueprint-gallery-status" class="config-note"></div>
          <menu>
            <button value="cancel" data-i18n="chrome.cancel">Cancel</button>
          </menu>
        </form>
      </dialog>
      <dialog id="jobs-runs">
        <form method="dialog">
          <h2 id="jobs-runs-title"></h2>
          <div id="jobs-runs-list" class="jobs-runs-list"></div>
          <menu>
            <button value="cancel" data-i18n="chrome.cancel">Cancel</button>
          </menu>
        </form>
      </dialog>
      <dialog id="jobs-blueprint-form">
        <form method="dialog">
          <h2 id="jobs-blueprint-form-title"></h2>
          <p id="jobs-blueprint-form-desc" class="jobs-blueprint-desc"></p>
          <div id="jobs-blueprint-fields" class="jobs-blueprint-fields"></div>
          <div id="jobs-blueprint-form-status" class="config-note"></div>
          <menu>
            <button value="cancel" data-i18n="chrome.cancel">Cancel</button>
            <button id="jobs-blueprint-create" value="create" data-i18n="jobs.blueprintCreate">Create job</button>
          </menu>
        </form>
      </dialog>`;

    (this.root.querySelector("#jobs-refresh") as HTMLButtonElement).onclick = () =>
      void this.refresh();
    const sortSelect = this.root.querySelector("#jobs-sort") as HTMLSelectElement;
    sortSelect.value = this.sortMode;
    sortSelect.addEventListener("change", () => {
      window.localStorage.setItem(SORT_KEY, sortSelect.value);
      this.render();
    });
    // P536: live job filter — re-renders the cached jobs client-side.
    this.root.querySelector("#jobs-filter")!.addEventListener("input", () => {
      this.filterText = (
        (this.root.querySelector("#jobs-filter") as HTMLInputElement).value || ""
      )
        .trim()
        .toLowerCase();
      this.render();
    });
    (this.root.querySelector("#jobs-new") as HTMLButtonElement).onclick = () =>
      this.openCreateDialog();
    // P673: suggestions panel — seed the catalog, accept/dismiss rows.
    (this.root.querySelector("#jobs-ideas") as HTMLButtonElement).onclick = () =>
      void this.seedSuggestionCatalog();
    this.root.querySelector("#jobs-suggestions-list")!.addEventListener("click", (event) => {
      const target = (event.target as HTMLElement).closest("button[data-ref]");
      if (!target) return;
      const ref = target.getAttribute("data-ref") || "";
      if (target.classList.contains("jobs-suggestion-accept")) {
        void this.acceptSuggestion(ref);
      } else if (target.classList.contains("jobs-suggestion-dismiss")) {
        void this.dismissSuggestion(ref);
      }
    });
    // P655: automation-blueprint gallery + slot form (hermes blueprint
    // catalog parity).
    (this.root.querySelector("#jobs-blueprints") as HTMLButtonElement).onclick = () =>
      void this.openBlueprintGallery();
    this.root.querySelector("#jobs-blueprint-filter")!.addEventListener("input", () => {
      const filter = (
        this.root.querySelector("#jobs-blueprint-filter") as HTMLInputElement
      ).value;
      this.renderBlueprintCards(filter);
    });
    (this.root.querySelector("#jobs-blueprint-create") as HTMLButtonElement).onclick = (
      event,
    ) => {
      event.preventDefault();
      void this.instantiateActiveBlueprint();
    };
    const dialog = this.root.querySelector("#job-create") as HTMLDialogElement;
    dialog.addEventListener("close", () => {
      if (dialog.returnValue !== "save") return;
      void this.createJob();
    });
  }

  // ---- P655: automation blueprints -----------------------------------

  private async openBlueprintGallery(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const dialog = this.root.querySelector("#jobs-blueprint-gallery") as HTMLDialogElement;
    const status = this.root.querySelector("#jobs-blueprint-gallery-status")!;
    const filterInput = this.root.querySelector(
      "#jobs-blueprint-filter",
    ) as HTMLInputElement;
    filterInput.value = "";
    status.textContent = "";
    this.root.querySelector("#jobs-blueprint-cards")!.innerHTML = "";
    if (!dialog.open) dialog.showModal();
    if (this.blueprints.length === 0) {
      try {
        this.blueprints = await client.jobsBlueprints();
      } catch (error) {
        status.textContent = fmt(t.jobs.blueprintsFailed, { error });
        return;
      }
    }
    this.renderBlueprintCards("");
  }

  private renderBlueprintCards(filter: string): void {
    const wrap = this.root.querySelector("#jobs-blueprint-cards")!;
    const status = this.root.querySelector("#jobs-blueprint-gallery-status")!;
    wrap.innerHTML = "";
    const query = (filter || "").trim().toLowerCase();
    const visible = query
      ? this.blueprints.filter(
          (bp) =>
            bp.title.toLowerCase().includes(query) ||
            bp.description.toLowerCase().includes(query) ||
            bp.category.toLowerCase().includes(query) ||
            bp.tags.some((tag) => tag.toLowerCase().includes(query)),
        )
      : this.blueprints;
    if (visible.length === 0) {
      status.textContent = t.jobs.blueprintsEmpty;
      return;
    }
    status.textContent = "";
    for (const blueprint of visible) {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "jobs-blueprint-card";
      const title = document.createElement("div");
      title.className = "jobs-blueprint-card-title";
      title.textContent = blueprint.title;
      const when = document.createElement("span");
      when.className = "jobs-blueprint-card-when";
      when.textContent = blueprint.scheduleHuman;
      title.appendChild(when);
      const desc = document.createElement("div");
      desc.className = "jobs-blueprint-card-desc";
      desc.textContent = blueprint.description;
      const meta = document.createElement("div");
      meta.className = "jobs-blueprint-card-meta";
      meta.textContent = [blueprint.category, ...blueprint.tags].join(" · ");
      card.appendChild(title);
      card.appendChild(desc);
      card.appendChild(meta);
      card.onclick = () => this.openBlueprintForm(blueprint);
      wrap.appendChild(card);
    }
  }

  private openBlueprintForm(blueprint: JobBlueprint): void {
    this.activeBlueprint = blueprint;
    const gallery = this.root.querySelector(
      "#jobs-blueprint-gallery",
    ) as HTMLDialogElement;
    gallery.close();
    const dialog = this.root.querySelector("#jobs-blueprint-form") as HTMLDialogElement;
    this.root.querySelector("#jobs-blueprint-form-title")!.textContent = blueprint.title;
    this.root.querySelector("#jobs-blueprint-form-desc")!.textContent = blueprint.description;
    this.root.querySelector("#jobs-blueprint-form-status")!.textContent = "";
    const fields = this.root.querySelector("#jobs-blueprint-fields")!;
    fields.innerHTML = "";
    for (const field of blueprint.fields) {
      const label = document.createElement("label");
      const caption = document.createElement("span");
      caption.textContent = field.label;
      label.appendChild(caption);
      let control: HTMLInputElement | HTMLSelectElement;
      if (field.type === "enum" || field.type === "weekdays") {
        const select = document.createElement("select");
        for (const option of field.options) {
          const optionEl = document.createElement("option");
          optionEl.value = option;
          optionEl.textContent = option;
          select.appendChild(optionEl);
        }
        select.value = field.default || field.options[0] || "";
        control = select;
      } else if (field.type === "time") {
        const input = document.createElement("input");
        input.type = "time";
        input.value = field.default || "08:00";
        control = input;
      } else {
        const input = document.createElement("input");
        input.type = "text";
        input.value = field.default || "";
        control = input;
      }
      control.id = `jobs-blueprint-field-${field.name}`;
      if (field.help) control.title = field.help;
      label.appendChild(control);
      fields.appendChild(label);
    }
    if (!dialog.open) dialog.showModal();
  }

  private async instantiateActiveBlueprint(): Promise<void> {
    const client = this.client();
    const blueprint = this.activeBlueprint;
    if (!client || !blueprint) return;
    const status = this.root.querySelector("#jobs-blueprint-form-status")!;
    const values: Record<string, string> = {};
    for (const field of blueprint.fields) {
      const control = this.root.querySelector(
        `#jobs-blueprint-field-${field.name}`,
      ) as HTMLInputElement | HTMLSelectElement | null;
      if (control) values[field.name] = control.value;
    }
    const result = await client.jobsInstantiateBlueprint(blueprint.key, values);
    if (!result.ok) {
      status.textContent = result.error || t.jobs.createFailed;
      return;
    }
    (this.root.querySelector("#jobs-blueprint-form") as HTMLDialogElement).close();
    this.activeBlueprint = null;
    void this.refresh();
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

  /** P474: public so run-settle events can refresh immediately. */
  async refresh(): Promise<void> {
    const client = this.client();
    if (!client) return;
    this.jobs = await client.jobsList(true);
    void this.cacheDeliveryTargets();
    void this.refreshSuggestions();
    this.render();
  }

  /** P673: fetch pending suggestions (best-effort) and render the panel. */
  private async refreshSuggestions(): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      this.suggestions = await client.listJobSuggestions();
    } catch {
      this.suggestions = [];
    }
    this.renderSuggestions();
  }

  /** P673: render pending suggestions with accept/dismiss actions. */
  private renderSuggestions(): void {
    const section = this.root.querySelector("#jobs-suggestions") as HTMLElement;
    const list = this.root.querySelector("#jobs-suggestions-list") as HTMLElement;
    if (!section || !list) return;
    if (this.suggestions.length === 0) {
      section.hidden = true;
      list.innerHTML = "";
      return;
    }
    section.hidden = false;
    list.innerHTML = this.suggestions
      .map((suggestion) => {
        const title = suggestion.title.replace(/</g, "&lt;");
        const description = suggestion.description.replace(/</g, "&lt;");
        const schedule = suggestion.job_spec?.schedule || "";
        const source = suggestion.source.replace(/</g, "&lt;");
        return `
        <div class="jobs-suggestion-row">
          <div class="jobs-suggestion-main">
            <strong>${title}</strong>
            <span class="jobs-suggestion-meta">[${source}] ${schedule}</span>
            <div class="jobs-suggestion-desc">${description}</div>
          </div>
          <button class="jobs-suggestion-accept" data-ref="${suggestion.id}" data-i18n="jobs.suggestionsAccept">Accept</button>
          <button class="ghost jobs-suggestion-dismiss" data-ref="${suggestion.id}" data-i18n="jobs.suggestionsDismiss">Dismiss</button>
        </div>`;
      })
      .join("");
  }

  /** P673: accept a suggestion — schedules it into the cron store. */
  private async acceptSuggestion(reference: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    const result = await client.acceptJobSuggestion(reference);
    if (result.ok) {
      window.alert(
        t.jobs.suggestionsScheduled
          .replace("{name}", result.name || reference)
          .replace("{schedule}", result.schedule || ""),
      );
    } else {
      window.alert(result.error || "accept failed");
    }
    await this.refresh();
  }

  /** P673: dismiss a suggestion so it is never re-offered. */
  private async dismissSuggestion(reference: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    await client.dismissJobSuggestion(reference);
    await this.refresh();
  }

  /** P673: seed the curated starter catalog, then show what is pending. */
  private async seedSuggestionCatalog(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const result = await client.seedJobSuggestionCatalog();
    if (result.ok) {
      const created = result.created || [];
      window.alert(
        created.length > 0
          ? t.jobs.suggestionsSeeded.replace("{count}", String(created.length))
          : t.jobs.suggestionsSeedNone,
      );
    } else {
      window.alert(result.error || "seed failed");
    }
    await this.refresh();
  }

  /** P507: cache known delivery-target ids for edit-time validation. */
  private async cacheDeliveryTargets(): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      const targets = await client.jobDeliveryTargets();
      this.deliveryTargetIds = new Set(targets.map((target) => target.id));
    } catch {
      // keep the previous cache; validation stays advisory anyway
    }
  }

  /**
   * P507: save a job edit with inline delivery-target validation — an
   * unknown target (not `origin`/`all`, not a listed target, and no
   * listed platform prefix for `platform:chat[:thread]` forms) asks
   * for confirmation before saving.
   */
  private async updateJob(id: string, prompt: string, schedule: string, deliver: string | null): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (deliver && !this.isKnownDeliveryTarget(deliver)) {
      if (!window.confirm(t.jobs.deliverUnknownConfirm.replace("{target}", deliver))) return;
    }
    try {
      await client.jobUpdate(id, { prompt, schedule, deliver });
      await this.refresh();
    } catch {
      // surface via the next poll; createFailed-style toast not wired here
    }
  }

  private isKnownDeliveryTarget(deliver: string): boolean {
    if (deliver === "origin" || deliver === "all") return true;
    if (this.deliveryTargetIds.has(deliver)) return true;
    const base = deliver.split(":")[0];
    return this.deliveryTargetIds.has(base);
  }

  /** P674: show the tracked-run history of one job in a dialog. */
  private async openJobRuns(job: CronJob): Promise<void> {
    const client = this.client();
    const dialog = this.root.querySelector("#jobs-runs") as HTMLDialogElement;
    const title = this.root.querySelector("#jobs-runs-title") as HTMLElement;
    const list = this.root.querySelector("#jobs-runs-list") as HTMLElement;
    title.textContent = t.jobs.runsHistoryTitle.replace("{name}", job.name);
    list.innerHTML = '<p class="jobs-empty">…</p>';
    dialog.showModal();
    if (!client) return;
    try {
      const runs = await client.jobRuns(job.id, 50);
      if (!runs.length) {
        list.innerHTML = "";
        const empty = document.createElement("p");
        empty.className = "jobs-empty";
        empty.textContent = t.jobs.runsEmpty;
        list.appendChild(empty);
        return;
      }
      list.innerHTML = runs
        .map((run) => {
          const status = run.status.replace(/</g, "&lt;");
          const when = new Date(run.created_at * 1000).toLocaleString();
          const duration = run.finished_at
            ? `${Math.max(0, Math.round(run.finished_at - run.created_at))}s`
            : "—";
          const error = run.error
            ? `<div class="job-meta job-delivery-error">${run.error.replace(/</g, "&lt;")}</div>`
            : "";
          return `<div class="job-row">
            <div class="job-body">
              <div class="job-name-line">
                <span class="job-name">${status}</span>
                <span class="job-schedule">${when}</span>
                <span class="job-skills">${duration}</span>
              </div>
              ${error}
            </div>
          </div>`;
        })
        .join("");
    } catch (error) {
      list.innerHTML = "";
      const failed = document.createElement("p");
      failed.className = "jobs-empty";
      failed.textContent = String(error);
      list.appendChild(failed);
    }
  }

  /**
   * Open the create dialog, optionally prefilled from an existing job so it
   * can be duplicated with tweaks (P549).
   */
  private openCreateDialog(source?: CronJob): void {
    (this.root.querySelector("#job-create-name") as HTMLInputElement).value = source
      ? source.name + t.jobs.duplicateSuffix
      : "";
    (this.root.querySelector("#job-create-schedule") as HTMLInputElement).value =
      source?.schedule ?? "";
    (this.root.querySelector("#job-create-prompt") as HTMLTextAreaElement).value =
      source?.prompt ?? "";
    (this.root.querySelector("#job-create-skills") as HTMLInputElement).value =
      source?.skills.join(", ") ?? "";
    (this.root.querySelector("#job-create-repeat") as HTMLInputElement).value =
      source && source.repeat !== null ? String(source.repeat) : "";
    void this.populateDeliverTargets().then(() => {
      if (!source?.deliver) return;
      const select = this.root.querySelector("#job-create-deliver") as HTMLSelectElement;
      const deliver = source.deliver;
      if ([...select.options].some((option) => option.value === deliver)) select.value = deliver;
    });
    (this.root.querySelector("#job-create") as HTMLDialogElement).showModal();
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

  /** Persisted job ordering (P506): "default" (API order) or "next_run". */
  private get sortMode(): string {
    return window.localStorage.getItem(SORT_KEY) ?? "default";
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
    const ordered = [...this.jobs];
    if (this.sortMode === "next_run") {
      ordered.sort((a, b) => {
        const nextA = a.next_run ?? Number.MAX_SAFE_INTEGER;
        const nextB = b.next_run ?? Number.MAX_SAFE_INTEGER;
        if (nextA !== nextB) return nextA - nextB;
        return a.name.localeCompare(b.name);
      });
    }
    // P536: narrow by name, schedule, prompt, or skill as you type.
    const query = this.filterText;
    const visible = query
      ? ordered.filter(
          (job) =>
            job.name.toLowerCase().includes(query) ||
            job.schedule.toLowerCase().includes(query) ||
            job.prompt.toLowerCase().includes(query) ||
            job.skills.some((skill) => skill.toLowerCase().includes(query)),
        )
      : ordered;
    if (!visible.length) {
      list.innerHTML = '<p class="jobs-empty"></p>';
      list.querySelector(".jobs-empty")!.textContent = t.jobs.filterNoMatch;
      return;
    }
    for (const job of visible) {
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
      deliveryError.innerHTML = `${ICON.warn} ${t.jobs.deliveryError}`;
      body.appendChild(deliveryError);
    }
    row.appendChild(body);

    const actions = document.createElement("div");
    actions.className = "job-actions";
    const mk = (label: string, title: string, onclick: () => void, danger = false): void => {
      const button = document.createElement("button");
      button.className = danger ? "ghost danger" : "ghost";
      button.innerHTML = label;
      button.title = title;
      button.onclick = onclick;
      actions.appendChild(button);
    };
    mk(job.enabled ? ICON.pause : ICON.play, job.enabled ? t.jobs.pause : t.jobs.resume, () => {
      if (!client) return;
      void client.jobSetEnabled(job.id, !job.enabled).then(() => void this.refresh());
    });
    mk(ICON.zap, t.jobs.runNow, () => {
      if (!client) return;
      void client.jobRunNow(job.id).then(() => void this.refresh());
    });
    // P674: per-job tracked-run history dialog.
    mk(ICON.list, t.jobs.runsHistory, () => void this.openJobRuns(job));
    mk(ICON.copy, t.jobs.duplicate, () => this.openCreateDialog(job));
    mk(ICON.pencil, t.jobs.edit, () => {
      if (!client) return;
      const nextPrompt = window.prompt(t.jobs.promptPrompt, job.prompt);
      if (nextPrompt === null) return;
      const nextSchedule = window.prompt(t.jobs.schedulePrompt, job.schedule);
      if (nextSchedule === null) return;
      const nextDeliver = window.prompt(t.jobs.deliverPrompt, job.deliver || "");
      if (nextDeliver === null) return;
      void this.updateJob(job.id, nextPrompt, nextSchedule, nextDeliver.trim() || null);
    });
    mk(ICON.trash, t.jobs.delete, () => {
      if (!client) return;
      if (!window.confirm(fmt(t.jobs.deleteConfirm, { name: job.name }))) return;
      void client.jobDelete(job.id).then(() => void this.refresh());
    }, true);
    row.appendChild(actions);
    return row;
  }
}
