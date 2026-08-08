// Plugins view — plugin inventory over `/api/plugins` (hermes `plugins
// list` parity): plugin rows with version, description, hook/tool
// counts, disabled badge and enable/disable toggles, plus the `[hooks]`
// config shell-hook listing. Install/update/remove stay in the CLI.

import type { GatewayClient, PluginsHubPayload, PluginsPayload, ToolsetRow } from "./gateway";
import { t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class PluginsViewWidget {
  private hub: PluginsHubPayload | null = null;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="plugins-view-header">
        <span id="plugins-view-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <input id="plugins-view-filter" type="search" data-i18n-ph="pluginsView.filterPlaceholder" />
        <button id="plugins-view-rescan" class="ghost" data-i18n="pluginsView.rescan">Rescan</button>
        <button id="plugins-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="plugins-view-status" class="config-status" hidden></div>
      <div id="plugins-view-list" class="skills-list"></div>
      <h3 class="config-section" data-i18n="pluginsView.hubTitle">Plugin hub</h3>
      <div id="plugins-view-hub"></div>
      <h3 class="config-section" data-i18n="pluginsView.providersTitle">Plugin providers</h3>
      <div id="plugins-view-providers"></div>
      <h3 class="config-section" data-i18n="pluginsView.configHooksTitle">Config shell hooks</h3>
      <div id="plugins-view-hooks"></div>
      <h3 class="config-section" data-i18n="pluginsView.toolsetsTitle">Toolsets</h3>
      <div id="plugins-view-toolsets"></div>
    `;
    this.root.querySelector("#plugins-view-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#plugins-view-rescan")!.addEventListener("click", () => {
      this.rescan().catch(() => undefined);
    });
    // P533: live filter across plugin, hub, and toolset rows.
    this.root.querySelector("#plugins-view-filter")!.addEventListener("input", () => {
      this.applyFilter();
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

  /** P533: narrow every `.plugins-view-row` (installed plugins, hub
   * catalog entries, toolsets) by name/description text as you type. */
  private applyFilter(): void {
    const input = this.root.querySelector("#plugins-view-filter") as HTMLInputElement | null;
    if (!input) return;
    const query = (input.value || "").trim().toLowerCase();
    let visible = 0;
    for (const row of Array.from(
      this.root.querySelectorAll<HTMLElement>(".plugins-view-row"),
    )) {
      const match = query === "" || (row.textContent || "").toLowerCase().includes(query);
      row.hidden = !match;
      if (match) visible += 1;
    }
    if (query === "") {
      this.status("");
    } else if (visible === 0) {
      this.status(t.pluginsView.filterNoMatch);
    } else {
      this.status("");
    }
  }

  async refresh(): Promise<void> {
    const client = this.client();
    if (!client) {
      this.status(t.config.notConnected, true);
      return;
    }
    try {
      const [payload, hub] = await Promise.all([
        client.pluginsInventory(),
        client.pluginsHub().catch(() => null),
      ]);
      this.hub = hub;
      this.render(payload);
      this.renderHub(hub);
      this.renderToolsets().catch(() => undefined);
      this.status("");
      this.applyFilter();
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
          const isHidden = this.hub?.hidden.includes(plugin.name) ?? false;
          return `
            <div class="plugins-view-row" data-name="${escapeHtml(plugin.name)}">
              <div class="plugins-view-row-head">
                <strong>${escapeHtml(plugin.name)}</strong> ${badges} ${disabledBadge}
                <span class="spacer"></span>
                <button class="ghost plugins-view-visibility">${escapeHtml(isHidden ? v.show : v.hide)}</button>
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
        row.querySelector(".plugins-view-visibility")!.addEventListener("click", () => {
          const hidden = this.hub?.hidden.includes(plugin.name) ?? false;
          this.setVisibility(plugin.name, !hidden).catch(() => undefined);
        });
      }
    }

    hooks.innerHTML = "";
    this.renderHooksConsent(hooks).catch(() => {
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
    });
  }

  /** Consent-aware hooks census over /api/ops/hooks (P326). */
  private async renderHooksConsent(hooks: HTMLElement): Promise<void> {
    const client = this.client();
    if (!client) throw new Error("offline");
    const v = t.pluginsView;
    const consent = await client.hooksConsent();
    hooks.innerHTML = "";
    if (consent.hooks.length === 0) {
      const empty = document.createElement("p");
      empty.className = "empty";
      empty.textContent = v.noConfigHooks;
      hooks.appendChild(empty);
    }
    for (const hook of consent.hooks) {
      const row = document.createElement("div");
      row.className = "monitoring-row";
      row.innerHTML = `
        <span class="monitoring-label">${escapeHtml(hook.event)}</span>
        <span class="monitoring-value">${escapeHtml(hook.command)}</span>
        <span class="jobs-counts">${escapeHtml(hook.state)}</span>
      `;
      if (hook.consented) {
        const revoke = document.createElement("button");
        revoke.className = "ghost danger";
        revoke.textContent = v.hooksRevoke;
        revoke.addEventListener("click", () => {
          client
            .hooksRevoke(hook.command)
            .then(() => this.refresh())
            .catch(() => undefined);
        });
        row.appendChild(revoke);
      }
      hooks.appendChild(row);
    }
    const footer = document.createElement("div");
    footer.className = "monitoring-row";
    const pending = consent.hooks.some((hook) => hook.state === "pending");
    footer.innerHTML = `<span class="monitoring-value">${escapeHtml(
      v.hooksAllowlist.replace("{count}", String(consent.allowlist.entries)),
    )}</span>`;
    if (pending) {
      const accept = document.createElement("button");
      accept.className = "ghost";
      accept.textContent = v.hooksAcceptAll;
      accept.addEventListener("click", () => {
        client
          .hooksAcceptAll()
          .then(() => this.refresh())
          .catch(() => undefined);
      });
      footer.appendChild(accept);
    }
    hooks.appendChild(footer);
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

  private fail(error: unknown): void {
    this.status(error instanceof Error ? error.message : String(error), true);
  }

  private renderHub(hub: PluginsHubPayload | null): void {
    const el = this.root.querySelector("#plugins-view-hub") as HTMLElement;
    const providers = this.root.querySelector("#plugins-view-providers") as HTMLElement;
    const v = t.pluginsView;
    if (!hub) {
      el.innerHTML = "";
      providers.innerHTML = "";
      return;
    }

    if (hub.catalog.length === 0) {
      el.innerHTML = `<p class="empty">${escapeHtml(v.hubNone)}</p>`;
    } else {
      el.innerHTML = hub.catalog
        .map((entry) => {
          const tags = entry.tags
            .map((tag) => `<span class="models-view-badge">${escapeHtml(tag)}</span>`)
            .join(" ");
          const actions = entry.installed
            ? `<span class="models-view-badge ok">${escapeHtml(v.hubInstalled)}</span>
               <button class="ghost plugins-view-hub-update">${escapeHtml(v.hubUpdate)}</button>
               <button class="ghost danger plugins-view-hub-remove">${escapeHtml(v.hubRemove)}</button>`
            : `<button class="ghost plugins-view-hub-install">${escapeHtml(v.hubInstall)}</button>`;
          return `
            <div class="plugins-view-row" data-name="${escapeHtml(entry.name)}">
              <div class="plugins-view-row-head">
                <strong>${escapeHtml(entry.name)}</strong>
                <span class="plugins-view-meta">${escapeHtml(entry.version || "—")}</span>
                <span class="plugins-view-meta">${escapeHtml(entry.source)}</span> ${tags}
                <span class="spacer"></span> ${actions}
              </div>
              ${entry.description ? `<div class="plugins-view-desc">${escapeHtml(entry.description)}</div>` : ""}
              <div class="plugins-view-sub muted">${escapeHtml(entry.identifier)}</div>
            </div>`;
        })
        .join("");
      for (const row of Array.from(el.querySelectorAll<HTMLElement>(".plugins-view-row"))) {
        const name = row.dataset.name || "";
        const entry = hub.catalog.find((candidate) => candidate.name === name);
        if (!entry) continue;
        const install = row.querySelector<HTMLButtonElement>(".plugins-view-hub-install");
        if (install) {
          install.addEventListener("click", () => this.install(entry.identifier, install));
        }
        row.querySelector(".plugins-view-hub-update")?.addEventListener("click", () => {
          this.updatePlugin(entry.name).catch(() => undefined);
        });
        row.querySelector(".plugins-view-hub-remove")?.addEventListener("click", () => {
          this.removePlugin(entry.name).catch(() => undefined);
        });
      }
    }

    providers.innerHTML = "";
    const memorySelect = this.providerSelect(hub.providers.memory, hub.selected.memory_provider);
    const contextSelect = this.providerSelect(hub.providers.context, hub.selected.context_engine);
    const rowOne = document.createElement("div");
    rowOne.className = "monitoring-row";
    rowOne.innerHTML = `<span class="monitoring-label">${escapeHtml(v.memoryProvider)}</span>`;
    rowOne.appendChild(memorySelect);
    const rowTwo = document.createElement("div");
    rowTwo.className = "monitoring-row";
    rowTwo.innerHTML = `<span class="monitoring-label">${escapeHtml(v.contextEngine)}</span>`;
    rowTwo.appendChild(contextSelect);
    const save = document.createElement("button");
    save.className = "ghost";
    save.textContent = v.providersSave;
    save.addEventListener("click", () => {
      const client = this.client();
      if (!client) return;
      client
        .setPluginProviders(memorySelect.value, contextSelect.value)
        .then(() => {
          this.status(`${v.providersSave} ✓`);
          return this.refresh();
        })
        .catch((error) => this.fail(error));
    });
    rowTwo.appendChild(save);
    providers.appendChild(rowOne);
    providers.appendChild(rowTwo);
  }

  private providerSelect(options: string[], selected: string): HTMLSelectElement {
    const select = document.createElement("select");
    for (const option of options) {
      const el = document.createElement("option");
      el.value = option;
      el.textContent = option;
      if (option === selected) el.selected = true;
      select.appendChild(el);
    }
    return select;
  }

  /** Toolsets panel (P354): /v1/toolsets groups with enablement posture. */
  private async renderToolsets(): Promise<void> {
    const client = this.client();
    const el = this.root.querySelector("#plugins-view-toolsets") as HTMLElement;
    const v = t.pluginsView;
    if (!client) {
      el.innerHTML = "";
      return;
    }
    let toolsets: ToolsetRow[];
    try {
      toolsets = await client.toolsetsList();
    } catch {
      el.innerHTML = "";
      return;
    }
    if (toolsets.length === 0) {
      el.innerHTML = `<p class="empty">${escapeHtml(v.toolsetsNone)}</p>`;
      return;
    }
    el.innerHTML = toolsets
      .map((toolset) => {
        const badge = toolset.enabled
          ? ""
          : `<span class="models-view-badge warn">${escapeHtml(v.disabledBadge)}</span>`;
        return `
          <details class="plugins-view-row">
            <summary>
              <strong>${escapeHtml(toolset.name)}</strong>
              <span class="plugins-view-meta">${toolset.tools.length} ${escapeHtml(v.toolsWord)}</span> ${badge}
              ${toolset.description ? `<span class="plugins-view-desc">${escapeHtml(toolset.description)}</span>` : ""}
            </summary>
            <div class="plugins-view-sub muted">${escapeHtml(toolset.tools.join(", "))}</div>
          </details>`;
      })
      .join("");
    // P533: toolset rows re-render asynchronously — reapply the filter.
    this.applyFilter();
  }

  private async rescan(): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      const count = await client.dashboardPluginsRescan();
      this.status(`${t.pluginsView.rescan}: ${count}`);
      await this.refresh();
    } catch (error) {
      this.fail(error);
    }
  }

  private async install(identifier: string, button: HTMLButtonElement): Promise<void> {
    const client = this.client();
    if (!client) return;
    const v = t.pluginsView;
    button.disabled = true;
    button.textContent = v.hubInstalling;
    try {
      const name = await client.agentPluginInstall(identifier);
      this.status(`${v.hubInstalled}: ${name}`);
      await this.refresh();
    } catch (error) {
      button.disabled = false;
      button.textContent = v.hubInstall;
      this.fail(error);
    }
  }

  private async updatePlugin(name: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      await client.agentPluginUpdate(name);
      this.status(`${t.pluginsView.hubUpdate}: ${name}`);
      await this.refresh();
    } catch (error) {
      this.fail(error);
    }
  }

  private async removePlugin(name: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      await client.agentPluginRemove(name);
      this.status(`${t.pluginsView.hubRemove}: ${name}`);
      await this.refresh();
    } catch (error) {
      this.fail(error);
    }
  }

  private async setVisibility(name: string, hidden: boolean): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      await client.setPluginVisibility(name, hidden);
      await this.refresh();
    } catch (error) {
      this.fail(error);
    }
  }
}
