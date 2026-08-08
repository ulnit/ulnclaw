// Profiles view — `[profiles.*]` config-override management over
// `/api/profiles*` (hermes desktop profiles parity, lean port over
// ulnclaw's config-override profiles): list, create, edit (model +
// toolset overrides), rename and delete.

import type { GatewayClient, ProfileRow } from "./gateway";
import { t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;

export class ProfilesViewWidget {
  private rows: ProfileRow[] = [];
  private multiplex = false;
  private configPath = "";
  private editing: string | null = null;
  private renameTarget: string | null = null;
  private deleteTarget: string | null = null;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="profiles-view-header">
        <span id="profiles-view-count" class="jobs-counts"></span>
        <span id="profiles-view-multiplex" class="config-note"></span>
        <span class="spacer"></span>
        <button id="profiles-view-new" class="ghost" data-i18n="profilesView.new">New profile</button>
        <button id="profiles-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="profiles-view-status" class="config-status" hidden></div>
      <div id="profiles-view-editor" class="profiles-view-editor" hidden>
        <h3 id="profiles-view-editor-title"></h3>
        <label><span data-i18n="profilesView.nameLabel">Name</span>
          <input id="profiles-editor-name" type="text" autocomplete="off" spellcheck="false" data-i18n-ph="profilesView.namePlaceholder" />
        </label>
        <label><span data-i18n="profilesView.providerLabel">Provider</span>
          <input id="profiles-editor-provider" type="text" autocomplete="off" spellcheck="false" placeholder="openai" />
        </label>
        <label><span data-i18n="profilesView.modelLabel">Model</span>
          <input id="profiles-editor-model" type="text" autocomplete="off" spellcheck="false" placeholder="gpt-4.1" />
        </label>
        <label><span data-i18n="profilesView.baseUrlLabel">Base URL (optional)</span>
          <input id="profiles-editor-base-url" type="text" autocomplete="off" spellcheck="false" placeholder="https://…" />
        </label>
        <label><span data-i18n="profilesView.temperatureLabel">Temperature (optional)</span>
          <input id="profiles-editor-temperature" type="text" autocomplete="off" spellcheck="false" placeholder="0.7" />
        </label>
        <label><span data-i18n="profilesView.enabledLabel">Enabled toolsets</span>
          <input id="profiles-editor-enabled" type="text" autocomplete="off" spellcheck="false" placeholder="terminal, web" />
        </label>
        <label><span data-i18n="profilesView.disabledLabel">Disabled toolsets</span>
          <input id="profiles-editor-disabled" type="text" autocomplete="off" spellcheck="false" placeholder="browser" />
        </label>
        <menu>
          <button id="profiles-editor-cancel" data-i18n="profilesView.cancel">Cancel</button>
          <button id="profiles-editor-save" value="default" data-i18n="profilesView.save">Save</button>
        </menu>
      </div>
      <div id="profiles-view-body"></div>
      <p id="profiles-view-note" class="config-note" data-i18n="profilesView.restartNote" hidden></p>
      <dialog id="profiles-rename-dialog">
        <h2 id="profiles-rename-title" data-i18n="profilesView.dialogRenameTitle">Rename profile</h2>
        <label><span data-i18n="profilesView.nameLabel">Name</span>
          <input id="profiles-rename-input" type="text" autocomplete="off" spellcheck="false" />
        </label>
        <p id="profiles-rename-status" class="config-status" hidden></p>
        <menu>
          <button id="profiles-rename-cancel" data-i18n="profilesView.cancel">Cancel</button>
          <button id="profiles-rename-save" value="default" data-i18n="profilesView.save">Save</button>
        </menu>
      </dialog>
      <dialog id="profiles-delete-dialog">
        <h2 id="profiles-delete-title" data-i18n="profilesView.del">Delete</h2>
        <p id="profiles-delete-prompt"></p>
        <menu>
          <button id="profiles-delete-cancel" data-i18n="profilesView.cancel">Cancel</button>
          <button id="profiles-delete-confirm" class="danger" data-i18n="profilesView.del">Delete</button>
        </menu>
      </dialog>
    `;
    this.root.querySelector("#profiles-view-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#profiles-view-new")!.addEventListener("click", () => {
      this.openEditor(null);
    });
    this.root.querySelector("#profiles-editor-cancel")!.addEventListener("click", () => {
      this.closeEditor();
    });
    this.root.querySelector("#profiles-editor-save")!.addEventListener("click", () => {
      this.saveEditor().catch(() => undefined);
    });
    this.root.querySelector("#profiles-rename-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#profiles-rename-dialog") as HTMLDialogElement).close();
    });
    this.root.querySelector("#profiles-rename-save")!.addEventListener("click", () => {
      this.commitRename().catch(() => undefined);
    });
    this.root.querySelector("#profiles-delete-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#profiles-delete-dialog") as HTMLDialogElement).close();
    });
    this.root.querySelector("#profiles-delete-confirm")!.addEventListener("click", () => {
      this.commitDelete().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* on-demand only */
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#profiles-view-status") as HTMLElement;
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
      const result = await client.profilesList();
      this.rows = result.profiles;
      this.multiplex = result.multiplex_profiles;
      this.configPath = result.path;
      this.render();
      this.status("");
    } catch (error) {
      this.status(
        t.profilesView.loadFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private render(): void {
    const count = this.root.querySelector("#profiles-view-count") as HTMLElement;
    count.textContent = t.profilesView.count.replace("{count}", String(this.rows.length));
    const multiplex = this.root.querySelector("#profiles-view-multiplex") as HTMLElement;
    multiplex.textContent = this.configPath
      ? `${t.profilesView.multiplex.replace("{state}", this.multiplex ? t.profilesView.multiplexOn : t.profilesView.multiplexOff)} · ${this.configPath}`
      : "";
    const note = this.root.querySelector("#profiles-view-note") as HTMLElement;
    note.hidden = this.rows.length === 0;

    const body = this.root.querySelector("#profiles-view-body") as HTMLElement;
    if (this.rows.length === 0) {
      body.innerHTML = `<p class="config-note">${escapeHtml(t.profilesView.empty)}</p>`;
      return;
    }
    body.innerHTML = this.rows
      .map((row) => {
        const model = row.model
          ? `${escapeHtml(row.model.provider)}:${escapeHtml(row.model.model)}`
          : escapeHtml(t.profilesView.modelNone);
        const chips: string[] = [];
        if (row.enabled_toolsets && row.enabled_toolsets.length > 0) {
          chips.push(
            `<span class="profiles-view-chip enabled">${escapeHtml(t.profilesView.enabled)}: ${escapeHtml(row.enabled_toolsets.join(", "))}</span>`,
          );
        }
        if (row.disabled_toolsets && row.disabled_toolsets.length > 0) {
          chips.push(
            `<span class="profiles-view-chip disabled">${escapeHtml(t.profilesView.disabled)}: ${escapeHtml(row.disabled_toolsets.join(", "))}</span>`,
          );
        }
        return `
          <div class="profiles-view-row" data-name="${escapeHtml(row.name)}">
            <div class="profiles-view-row-main">
              <strong>${escapeHtml(row.name)}</strong>
              <span class="profiles-view-model">${model}</span>
              ${chips.join("")}
            </div>
            <div class="profiles-view-row-actions">
              <button class="ghost" data-action="edit" title="${escapeHtml(t.profilesView.edit)}">✎</button>
              <button class="ghost" data-action="rename">${escapeHtml(t.profilesView.rename)}</button>
              <button class="ghost" data-action="delete" title="${escapeHtml(t.profilesView.del)}">🗑</button>
            </div>
          </div>`;
      })
      .join("");
    body.querySelectorAll<HTMLButtonElement>("button[data-action]").forEach((button) => {
      button.addEventListener("click", () => {
        const name = button.closest(".profiles-view-row")?.getAttribute("data-name") || "";
        const action = button.getAttribute("data-action");
        if (action === "edit") this.openEditor(name);
        else if (action === "rename") this.openRename(name);
        else if (action === "delete") this.openDelete(name);
      });
    });
  }

  private input(id: string): HTMLInputElement {
    return this.root.querySelector(`#${id}`) as HTMLInputElement;
  }

  private openEditor(name: string | null): void {
    this.editing = name;
    const editor = this.root.querySelector("#profiles-view-editor") as HTMLElement;
    const title = this.root.querySelector("#profiles-view-editor-title") as HTMLElement;
    title.textContent =
      name === null
        ? t.profilesView.dialogNewTitle
        : t.profilesView.dialogEditTitle.replace("{name}", name);
    const row = name === null ? null : this.rows.find((entry) => entry.name === name) || null;
    this.input("profiles-editor-name").value = row?.name || "";
    this.input("profiles-editor-name").disabled = row !== null;
    this.input("profiles-editor-provider").value = row?.model?.provider || "";
    this.input("profiles-editor-model").value = row?.model?.model || "";
    this.input("profiles-editor-base-url").value = row?.model?.base_url || "";
    this.input("profiles-editor-temperature").value =
      row?.model?.temperature !== null && row?.model?.temperature !== undefined
        ? String(row.model.temperature)
        : "";
    this.input("profiles-editor-enabled").value = (row?.enabled_toolsets || []).join(", ");
    this.input("profiles-editor-disabled").value = (row?.disabled_toolsets || []).join(", ");
    editor.hidden = false;
    this.input("profiles-editor-name").focus();
  }

  private closeEditor(): void {
    this.editing = null;
    (this.root.querySelector("#profiles-view-editor") as HTMLElement).hidden = true;
  }

  private parseToolsets(raw: string): string[] {
    return raw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
  }

  private async saveEditor(): Promise<void> {
    const client = this.client();
    if (!client) {
      this.status(t.config.notConnected, true);
      return;
    }
    const name = this.input("profiles-editor-name").value.trim();
    if (!NAME_RE.test(name)) {
      this.status(t.profilesView.invalidName, true);
      return;
    }
    const provider = this.input("profiles-editor-provider").value.trim();
    const model = this.input("profiles-editor-model").value.trim();
    const baseUrl = this.input("profiles-editor-base-url").value.trim();
    const temperatureRaw = this.input("profiles-editor-temperature").value.trim();
    const body: Parameters<GatewayClient["profileSave"]>[0] = {
      name,
      enabled_toolsets: this.parseToolsets(this.input("profiles-editor-enabled").value),
      disabled_toolsets: this.parseToolsets(this.input("profiles-editor-disabled").value),
    };
    if (provider || model) {
      body.provider = provider;
      body.model = model;
      if (baseUrl) body.base_url = baseUrl;
      if (temperatureRaw) {
        const parsed = Number(temperatureRaw);
        if (Number.isFinite(parsed)) body.temperature = parsed;
      }
    }
    try {
      const result = await client.profileSave(body);
      this.closeEditor();
      await this.refresh();
      this.status(t.profilesView.savedNote.replace("{name}", result.profile.name));
    } catch (error) {
      this.status(
        t.profilesView.saveFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private openRename(name: string): void {
    this.renameTarget = name;
    const dialog = this.root.querySelector("#profiles-rename-dialog") as HTMLDialogElement;
    const title = this.root.querySelector("#profiles-rename-title") as HTMLElement;
    title.textContent = t.profilesView.dialogRenameTitle.replace("{name}", name);
    const input = this.input("profiles-rename-input");
    input.value = name;
    (this.root.querySelector("#profiles-rename-status") as HTMLElement).hidden = true;
    dialog.showModal();
    input.focus();
    input.select();
  }

  private async commitRename(): Promise<void> {
    const client = this.client();
    const name = this.renameTarget;
    if (!client || !name) return;
    const newName = this.input("profiles-rename-input").value.trim();
    const statusEl = this.root.querySelector("#profiles-rename-status") as HTMLElement;
    if (!NAME_RE.test(newName)) {
      statusEl.hidden = false;
      statusEl.textContent = t.profilesView.invalidName;
      statusEl.classList.add("error");
      return;
    }
    try {
      const result = await client.profileRename(name, newName);
      (this.root.querySelector("#profiles-rename-dialog") as HTMLDialogElement).close();
      this.renameTarget = null;
      await this.refresh();
      this.status(t.profilesView.renamedNote.replace("{name}", result.name));
    } catch (error) {
      statusEl.hidden = false;
      statusEl.classList.add("error");
      statusEl.textContent = t.profilesView.renameFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private openDelete(name: string): void {
    this.deleteTarget = name;
    const prompt = this.root.querySelector("#profiles-delete-prompt") as HTMLElement;
    prompt.textContent = t.profilesView.confirmDelete.split("{name}").join(name);
    (this.root.querySelector("#profiles-delete-dialog") as HTMLDialogElement).showModal();
  }

  private async commitDelete(): Promise<void> {
    const client = this.client();
    const name = this.deleteTarget;
    if (!client || !name) return;
    try {
      await client.profileDelete(name);
      (this.root.querySelector("#profiles-delete-dialog") as HTMLDialogElement).close();
      this.deleteTarget = null;
      await this.refresh();
      this.status(t.profilesView.deletedNote.replace("{name}", name));
    } catch (error) {
      (this.root.querySelector("#profiles-delete-dialog") as HTMLDialogElement).close();
      this.status(
        t.profilesView.deleteFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }
}
