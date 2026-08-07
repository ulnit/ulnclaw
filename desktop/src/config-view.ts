// Config view — config.toml editor over the gateway `/api/config` API.
// Flattens the nested TOML into dotted-path rows grouped by top-level
// section, tracks pending edits locally, and parses input values the way
// `ulnclaw config set` does (JSON-scalar first, then raw string). Masked
// secret values round-trip untouched: the server skips the "[redacted]"
// placeholder on PUT.

import type { GatewayClient } from "./gateway";
import { t } from "./i18n";

const REDACTED = "[redacted]";

function toInputString(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  return JSON.stringify(value);
}

function parseInput(raw: string): unknown {
  const trimmed = raw.trim();
  if (trimmed === "") return "";
  try {
    return JSON.parse(trimmed);
  } catch {
    return raw;
  }
}

export class ConfigWidget {
  /** Baseline input strings by dotted path (as last loaded/saved). */
  private base = new Map<string, string>();
  /** Current input element values by dotted path. */
  private inputs = new Map<string, HTMLInputElement>();
  /** Paths removed since the last load/save (pending unset). */
  private removed = new Set<string>();
  private redacted = new Set<string>();
  private envKeys: string[] = [];
  private configPath = "";
  private busy = false;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="config-header">
        <span id="config-file" class="jobs-counts"></span>
        <span id="config-pending" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="config-raw" class="ghost" data-i18n="config.rawButton">Raw TOML…</button>
        <button id="config-reload" class="ghost" data-i18n="config.reload">Reload</button>
        <button id="config-save" class="primary" data-i18n="config.save">Save</button>
      </header>
      <div id="config-status" class="config-status" hidden></div>
      <div id="config-rows" class="config-rows"></div>
      <div id="config-add" class="config-add">
        <span class="config-add-label" data-i18n="config.addKey">Add key</span>
        <input id="config-add-key" type="text" data-i18n-ph="config.keyPlaceholder" placeholder="dotted.key.path" />
        <input id="config-add-value" type="text" data-i18n-ph="config.valuePlaceholder" placeholder="value (JSON or text)" />
        <button id="config-add-btn" class="ghost" data-i18n="config.add">Add</button>
      </div>
      <p class="config-note" data-i18n="config.redactedNote"></p>
      <p class="config-note" data-i18n="config.restartNote"></p>
      <div id="config-env" class="config-env" hidden>
        <h3 data-i18n="config.envKeys">Environment keys (.env)</h3>
        <div id="config-env-chips" class="config-env-chips"></div>
        <p class="config-note" data-i18n="config.envKeysNote"></p>
      </div>
      <dialog id="config-raw-dialog" class="config-raw-dialog">
        <h2 data-i18n="config.rawTitle">Raw config.toml</h2>
        <textarea id="config-raw-text" spellcheck="false"></textarea>
        <p id="config-raw-status" class="config-note"></p>
        <menu>
          <button id="config-raw-cancel" class="ghost" data-i18n="chrome.cancel">Cancel</button>
          <button id="config-raw-save" class="primary" data-i18n="config.rawSave">Save raw</button>
        </menu>
      </dialog>
    `;
    this.root.querySelector("#config-raw")!.addEventListener("click", () => {
      this.openRaw().catch(() => undefined);
    });
    this.root.querySelector("#config-raw-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#config-raw-dialog") as HTMLDialogElement).close();
    });
    this.root.querySelector("#config-raw-save")!.addEventListener("click", () => {
      this.saveRaw().catch(() => undefined);
    });
    this.root.querySelector("#config-reload")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#config-save")!.addEventListener("click", () => {
      this.save().catch(() => undefined);
    });
    this.root.querySelector("#config-add-btn")!.addEventListener("click", () => this.addKey());
    const valueInput = this.root.querySelector("#config-add-value") as HTMLInputElement;
    valueInput.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        this.addKey();
      }
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* nothing polls */
  }

  /** Raw TOML editor (P318): hermes /api/config/raw parity — the
   * flattened editor drops comments, this one keeps the file verbatim. */
  private async openRaw(): Promise<void> {
    const client = this.client();
    const dialog = this.root.querySelector("#config-raw-dialog") as HTMLDialogElement;
    const text = this.root.querySelector("#config-raw-text") as HTMLTextAreaElement;
    const status = this.root.querySelector("#config-raw-status") as HTMLElement;
    status.textContent = "";
    text.value = "";
    dialog.showModal();
    if (!client) {
      status.textContent = t.config.notConnected;
      return;
    }
    try {
      const raw = await client.configRaw();
      text.value = raw.toml;
    } catch (error) {
      status.textContent = t.config.loadFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private async saveRaw(): Promise<void> {
    const client = this.client();
    const text = this.root.querySelector("#config-raw-text") as HTMLTextAreaElement;
    const status = this.root.querySelector("#config-raw-status") as HTMLElement;
    if (!client) return;
    if (!window.confirm(t.config.rawConfirm)) return;
    try {
      await client.saveConfigRaw(text.value);
      (this.root.querySelector("#config-raw-dialog") as HTMLDialogElement).close();
      await this.refresh();
      this.status(t.config.rawSaved);
    } catch (error) {
      status.textContent = t.config.rawFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#config-status") as HTMLElement;
    el.hidden = !message;
    el.textContent = message;
    el.classList.toggle("error", isError);
  }

  private pendingCount(): number {
    let count = this.removed.size;
    for (const [path, input] of this.inputs) {
      if (input.value !== (this.base.get(path) ?? "")) count += 1;
    }
    return count;
  }

  private updatePendingBadge(): void {
    const count = this.pendingCount();
    const el = this.root.querySelector("#config-pending") as HTMLElement;
    el.textContent = count > 0 ? t.config.pending.replace("{count}", String(count)) : "";
  }

  async refresh(): Promise<void> {
    const client = this.client();
    if (!client) {
      this.status(t.config.notConnected, true);
      return;
    }
    this.status(t.config.loading);
    try {
      const payload = await client.configGet();
      this.base.clear();
      this.inputs.clear();
      this.removed.clear();
      this.redacted = new Set(payload.redacted);
      this.envKeys = payload.env_keys || [];
      this.configPath = payload.path;
      flatten(payload.config as Record<string, unknown>, "", this.base);
      this.renderRows();
      this.renderEnv();
      this.status("");
      const fileEl = this.root.querySelector("#config-file") as HTMLElement;
      fileEl.textContent = this.configPath;
      fileEl.title = this.configPath;
    } catch (error) {
      this.status(
        t.config.loadFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    }
  }

  private renderRows(): void {
    const container = this.root.querySelector("#config-rows") as HTMLElement;
    container.innerHTML = "";
    const paths = [...this.base.keys()].sort();
    if (paths.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.config.noKeys;
      container.appendChild(empty);
      this.updatePendingBadge();
      return;
    }
    let section = "";
    for (const path of paths) {
      const top = path.split(".")[0];
      if (top !== section) {
        section = top;
        const header = document.createElement("h3");
        header.className = "config-section";
        header.textContent = section;
        container.appendChild(header);
      }
      const row = document.createElement("div");
      row.className = "config-row";
      const label = document.createElement("label");
      label.className = "config-key";
      label.textContent = path;
      label.title = path;
      const input = document.createElement("input");
      input.type = "text";
      input.value = this.base.get(path) ?? "";
      input.className = "config-value";
      if (this.redacted.has(path)) {
        input.classList.add("redacted");
        input.title = t.config.redactedNote;
      }
      input.addEventListener("input", () => this.updatePendingBadge());
      const remove = document.createElement("button");
      remove.className = "ghost config-remove";
      remove.textContent = "✕";
      remove.title = t.config.removeTitle;
      remove.addEventListener("click", () => {
        this.removed.add(path);
        this.inputs.delete(path);
        this.base.delete(path);
        row.remove();
        this.updatePendingBadge();
      });
      row.appendChild(label);
      row.appendChild(input);
      row.appendChild(remove);
      container.appendChild(row);
      this.inputs.set(path, input);
    }
    this.updatePendingBadge();
  }

  private renderEnv(): void {
    const block = this.root.querySelector("#config-env") as HTMLElement;
    const chips = this.root.querySelector("#config-env-chips") as HTMLElement;
    chips.innerHTML = "";
    if (this.envKeys.length === 0) {
      block.hidden = true;
      return;
    }
    block.hidden = false;
    for (const key of this.envKeys) {
      const chip = document.createElement("span");
      chip.className = "config-env-chip";
      chip.textContent = key;
      chips.appendChild(chip);
    }
  }

  private addKey(): void {
    const keyEl = this.root.querySelector("#config-add-key") as HTMLInputElement;
    const valueEl = this.root.querySelector("#config-add-value") as HTMLInputElement;
    const key = keyEl.value.trim();
    if (!key || this.base.has(key)) {
      keyEl.focus();
      return;
    }
    this.base.set(key, valueEl.value);
    keyEl.value = "";
    valueEl.value = "";
    this.renderRows();
  }

  private async save(): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    const set: Record<string, unknown> = {};
    let setCount = 0;
    for (const [path, input] of this.inputs) {
      const baseline = this.base.get(path) ?? "";
      if (input.value !== baseline) {
        set[path] = parseInput(input.value);
        setCount += 1;
      }
    }
    const unset = [...this.removed];
    if (setCount === 0 && unset.length === 0) {
      this.status(t.config.noChanges);
      return;
    }
    this.busy = true;
    const saveBtn = this.root.querySelector("#config-save") as HTMLButtonElement;
    saveBtn.disabled = true;
    saveBtn.textContent = t.config.saving;
    try {
      await client.configSave(set, unset);
      this.status(
        t.config.saved.replace("{count}", String(setCount + unset.length)),
      );
      await this.refresh();
    } catch (error) {
      this.status(
        t.config.saveFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
      saveBtn.disabled = false;
      saveBtn.textContent = t.config.save;
    }
  }
}

/** Walk a nested config object into dotted-path leaf entries. */
function flatten(
  value: Record<string, unknown>,
  prefix: string,
  out: Map<string, string>,
): void {
  for (const [key, entry] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (entry !== null && typeof entry === "object" && !Array.isArray(entry)) {
      flatten(entry as Record<string, unknown>, path, out);
    } else {
      out.set(path, toInputString(entry));
    }
  }
}
