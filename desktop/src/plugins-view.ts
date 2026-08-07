// Plugins view — plugin inventory over `/api/plugins` (hermes `plugins
// list` parity): plugin rows with version, description, hook/tool
// counts, disabled badge and enable/disable toggles, plus the `[hooks]`
// config shell-hook listing. Install/update/remove stay in the CLI.

import type { GatewayClient, PluginsPayload } from "./gateway";
import { t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class PluginsViewWidget {
  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="plugins-view-header">
        <span id="plugins-view-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="plugins-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="plugins-view-status" class="config-status" hidden></div>
      <div id="plugins-view-list" class="skills-list"></div>
      <h3 class="config-section" data-i18n="pluginsView.configHooksTitle">Config shell hooks</h3>
      <div id="plugins-view-hooks"></div>
    `;
    this.root.querySelector("#plugins-view-refresh")!.addEventListener("click", () => {
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
    const el = this.root.querySelector("#plugins-view-status") as HTMLElement;
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
      const payload = await client.pluginsInventory();
      this.render(payload);
      this.status("");
    } catch (error) {
      this.status(
        t.pluginsView.loadFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private render(payload: PluginsPayload): void {
    const list = this.root.querySelector("#plugins-view-list") as HTMLElement;
    const hooks = this.root.querySelector("#plugins-view-hooks") as HTMLElement;
    const v = t.pluginsView;

    (this.root.querySelector("#plugins-view-count") as HTMLElement).textContent =
      v.count.replace("{count}", String(payload.plugins.length));

    if (payload.plugins.length === 0) {
      list.innerHTML = `<p class="empty">${escapeHtml(v.none)}</p>`;
    } else {
      list.innerHTML = payload.plugins
        .map((plugin) => {
          const badges = [
            `<span class="plugins-view-meta">${escapeHtml(plugin.version || "—")}</span>`,
            `<span class="plugins-view-meta">${plugin.hooks.length} ${escapeHtml(v.hooksWord)}</span>`,
            `<span class="plugins-view-meta">${plugin.tools.length} ${escapeHtml(v.toolsWord)}</span>`,
          ].join(" ");
          const disabledBadge = plugin.disabled
            ? `<span class="models-view-badge warn">${escapeHtml(v.disabledBadge)}</span>`
            : "";
          const toggleLabel = plugin.disabled ? v.enable : v.disable;
          return `
            <div class="plugins-view-row" data-name="${escapeHtml(plugin.name)}">
              <div class="plugins-view-row-head">
                <strong>${escapeHtml(plugin.name)}</strong> ${badges} ${disabledBadge}
                <span class="spacer"></span>
                <button class="ghost plugins-view-toggle">${escapeHtml(toggleLabel)}</button>
              </div>
              ${plugin.description ? `<div class="plugins-view-desc">${escapeHtml(plugin.description)}</div>` : ""}
              ${plugin.hooks.length ? `<div class="plugins-view-sub">${escapeHtml(v.hooksWord)}: ${escapeHtml(plugin.hooks.join(", "))}</div>` : ""}
              ${plugin.tools.length ? `<div class="plugins-view-sub">${escapeHtml(v.toolsWord)}: ${escapeHtml(plugin.tools.map((tool) => tool.name).join(", "))}</div>` : ""}
              <div class="plugins-view-sub muted">${escapeHtml(plugin.dir)}</div>
            </div>`;
        })
        .join("");
      for (const row of Array.from(list.querySelectorAll<HTMLElement>(".plugins-view-row"))) {
        const name = row.dataset.name || "";
        const plugin = payload.plugins.find((candidate) => candidate.name === name);
        if (!plugin) continue;
        row.querySelector(".plugins-view-toggle")!.addEventListener("click", () => {
          this.toggle(plugin.name, plugin.disabled).catch(() => undefined);
        });
      }
    }

    const events = Object.entries(payload.config_hooks || {});
    if (events.length === 0) {
      hooks.innerHTML = `<p class="empty">${escapeHtml(v.noConfigHooks)}</p>`;
    } else {
      hooks.innerHTML = events
        .map(
          ([event, commands]) => `
            <div class="monitoring-row">
              <span class="monitoring-label">${escapeHtml(event)}</span>
              <span class="monitoring-value">${escapeHtml(commands.join(" · "))}</span>
            </div>`,
        )
        .join("");
    }
  }

  private async toggle(name: string, currentlyDisabled: boolean): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      const message = currentlyDisabled
        ? await client.pluginEnable(name)
        : await client.pluginDisable(name);
      this.status(message, false);
      await this.refresh();
    } catch (error) {
      this.status(
        t.pluginsView.toggleFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }
}
