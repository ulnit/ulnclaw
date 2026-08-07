// Skills view — catalog of installed skills (`/v1/skills`) and toolsets
// (`/v1/toolsets`): skill cards with category + description + source
// path, and toolset rows showing enabled state with an expandable tool
// list. Read-only companion to the composer's `/` slash completion.

import type { GatewayClient } from "./gateway";
import { t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class SkillsWidget {
  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="skills-header">
        <span id="skills-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="skills-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="skills-status" class="config-status" hidden></div>
      <h3 class="config-section" data-i18n="skillsView.skillsTitle">Installed skills</h3>
      <div id="skills-list" class="skills-list"></div>
      <h3 class="config-section" data-i18n="skillsView.toolsetsTitle">Toolsets</h3>
      <div id="toolsets-list" class="skills-list"></div>
      <h3 class="config-section" data-i18n="skillsView.curationTitle">Curation</h3>
      <div id="curation-summary" class="config-note"></div>
      <div id="curation-list" class="skills-list"></div>
    `;
    this.root.querySelector("#skills-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* on-demand only */
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#skills-status") as HTMLElement;
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
      const [skills, toolsets] = await Promise.all([
        client.listSkills(),
        client.listToolsets(),
      ]);
      this.renderSkills(skills);
      this.renderToolsets(toolsets);
      const enabledCount = toolsets.filter((toolset) => toolset.enabled).length;
      (this.root.querySelector("#skills-count") as HTMLElement).textContent =
        t.skillsView.count
          .replace("{skills}", String(skills.length))
          .replace("{toolsets}", `${enabledCount}/${toolsets.length}`);
      this.status("");
      this.loadCuration().catch(() => undefined);
    } catch (error) {
      this.status(
        t.skillsView.loadFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    }
  }

  /** Curation section (P316): curator status, usage rows with
   * pin/archive actions, archived skills with restore — hermes
   * `ulnclaw curator` parity over /api/curator. */
  private async loadCuration(): Promise<void> {
    const client = this.client();
    const summary = this.root.querySelector("#curation-summary") as HTMLElement;
    const list = this.root.querySelector("#curation-list") as HTMLElement;
    if (!client) return;
    try {
      const data = await client.curatorStatus();
      summary.textContent = data.status
        .map((row) => `${row.label}: ${row.count}`)
        .join(" \u00b7 ");
      list.innerHTML = "";
      if (data.archived.length) {
        const header = document.createElement("div");
        header.className = "config-note";
        header.textContent = t.skillsView.archivedTitle;
        list.appendChild(header);
        for (const name of data.archived) {
          const card = document.createElement("div");
          card.className = "skill-card";
          const head = document.createElement("div");
          head.className = "skill-head";
          const nameEl = document.createElement("span");
          nameEl.className = "skill-name";
          nameEl.textContent = name;
          const restore = document.createElement("button");
          restore.className = "ghost";
          restore.textContent = t.skillsView.restoreSkill;
          restore.onclick = () => this.curatorAction("restore", name);
          head.append(nameEl, restore);
          card.appendChild(head);
          list.appendChild(card);
        }
      }
      for (const row of data.usage.slice(0, 60)) {
        const card = document.createElement("div");
        card.className = "skill-card";
        const head = document.createElement("div");
        head.className = "skill-head";
        const nameEl = document.createElement("span");
        nameEl.className = "skill-name";
        nameEl.textContent = row.name;
        const state = document.createElement("span");
        state.className = "skill-category";
        state.textContent = `${row.state}${row.pinned ? " \u00b7 \ud83d\udccc" : ""}`;
        const meta = document.createElement("span");
        meta.className = "jobs-counts";
        meta.textContent = `${row.activity_count} act \u00b7 ${row.use_count} use${
          row.last_activity_at ? ` \u00b7 ${row.last_activity_at.slice(0, 10)}` : ""
        }`;
        const pin = document.createElement("button");
        pin.className = "ghost";
        pin.textContent = row.pinned ? t.skillsView.unpinSkill : t.skillsView.pinSkill;
        pin.onclick = () => this.curatorAction(row.pinned ? "unpin" : "pin", row.name);
        head.append(nameEl, state, meta, pin);
        if (!row.pinned) {
          const archive = document.createElement("button");
          archive.className = "ghost danger";
          archive.textContent = t.skillsView.archiveSkill;
          archive.onclick = () => this.curatorAction("archive", row.name);
          head.appendChild(archive);
        }
        card.appendChild(head);
        list.appendChild(card);
      }
    } catch {
      summary.textContent = "";
      list.innerHTML = "";
    }
  }

  private async curatorAction(
    action: "pin" | "unpin" | "archive" | "restore",
    skill: string,
  ): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (action === "archive" && !window.confirm(t.skillsView.archiveConfirm.replace("{name}", skill))) {
      return;
    }
    try {
      await client.curatorAction(action, skill);
      await this.loadCuration();
    } catch (error) {
      this.status(
        t.skillsView.curationFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private renderSkills(skills: { name: string; description: string; category?: string; path?: string }[]): void {
    const list = this.root.querySelector("#skills-list") as HTMLElement;
    list.innerHTML = "";
    if (skills.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.skillsView.noSkills;
      list.appendChild(empty);
      return;
    }
    for (const skill of skills) {
      const card = document.createElement("div");
      card.className = "skill-card";
      card.innerHTML = `
        <div class="skill-head">
          <span class="skill-name">/${escapeHtml(skill.name)}</span>
          ${skill.category ? `<span class="skill-category">${escapeHtml(skill.category)}</span>` : ""}
        </div>
        ${skill.description ? `<div class="skill-desc">${escapeHtml(skill.description)}</div>` : ""}
        ${skill.path ? `<div class="skill-path"><code>${escapeHtml(skill.path)}</code></div>` : ""}
      `;
      list.appendChild(card);
    }
  }

  private renderToolsets(toolsets: { name: string; description: string; enabled: boolean; tools: string[] }[]): void {
    const list = this.root.querySelector("#toolsets-list") as HTMLElement;
    list.innerHTML = "";
    if (toolsets.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.skillsView.noToolsets;
      list.appendChild(empty);
      return;
    }
    for (const toolset of toolsets) {
      const card = document.createElement("div");
      card.className = "skill-card";
      const tools = toolset.tools.map((tool) => `<code>${escapeHtml(tool)}</code>`).join(" ");
      card.innerHTML = `
        <div class="skill-head">
          <span class="skill-name">${escapeHtml(toolset.name)}</span>
          <span class="toolset-state ${toolset.enabled ? "on" : "off"}">
            ${escapeHtml(toolset.enabled ? t.skillsView.enabled : t.skillsView.disabled)}
          </span>
          <span class="jobs-counts">${toolset.tools.length} tools</span>
        </div>
        ${toolset.description ? `<div class="skill-desc">${escapeHtml(toolset.description)}</div>` : ""}
        ${toolset.tools.length > 0 ? `<details class="toolset-tools"><summary>${escapeHtml(t.skillsView.tools)}</summary><div class="toolset-tools-list">${tools}</div></details>` : ""}
      `;
      list.appendChild(card);
    }
  }
}
