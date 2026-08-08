// Config view — config.toml editor over the gateway `/api/config` API.
// Flattens the nested TOML into dotted-path rows grouped by top-level
// section, tracks pending edits locally, and parses input values the way
// `ulnclaw config set` does (JSON-scalar first, then raw string). Masked
// secret values round-trip untouched: the server skips the "[redacted]"
// placeholder on PUT.

import type { GatewayClient, ProviderOAuthRow, UpdateCheckResult } from "./gateway";
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
  /** Pending provider-OAuth device-flow poll timer (P350). */
  private oauthPollTimer: number | null = null;

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
      <input id="config-filter" type="search" data-i18n-ph="config.filterKeysPlaceholder" />
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
        <div id="config-env-rows" class="config-env-rows"></div>
        <div class="config-add">
          <span class="config-add-label" data-i18n="config.envAddLabel">Add env key</span>
          <input id="config-env-add-key" type="text" placeholder="ALL_CAPS_KEY" />
          <input id="config-env-add-value" type="password" data-i18n-ph="config.envValuePlaceholder" placeholder="value (stored in .env)" />
          <button id="config-env-validate-btn" class="ghost" data-i18n="config.envValidate">Validate</button>
          <button id="config-env-add-btn" class="ghost" data-i18n="config.add">Add</button>
        </div>
        <p class="config-note" data-i18n="config.envKeysNote"></p>
      </div>
      <div id="config-schema" class="config-env" hidden>
        <h3 data-i18n="config.schemaTitle">Config schema (defaults)</h3>
        <div id="config-schema-rows" class="config-env-rows"></div>
        <p class="config-note" data-i18n="config.schemaNote"></p>
      </div>
      <div id="config-tts" class="config-env" hidden>
        <h3 data-i18n="config.ttsTitle">Text-to-speech</h3>
        <div id="config-tts-rows" class="config-env-rows"></div>
        <p class="config-note" data-i18n="config.ttsNote"></p>
      </div>
      <div id="config-memory" class="config-env" hidden>
        <h3 data-i18n="config.memoryTitle">Persistent memory</h3>
        <div id="config-memory-rows" class="config-env-rows"></div>
        <div class="config-add">
          <select id="config-memory-target">
            <option value="all" data-i18n="config.memoryTargetAll">All (MEMORY.md + USER.md)</option>
            <option value="memory" data-i18n="config.memoryTargetMemory">MEMORY.md only</option>
            <option value="user" data-i18n="config.memoryTargetUser">USER.md only</option>
          </select>
          <button id="config-memory-reset-btn" class="ghost danger" data-i18n="config.memoryReset">Reset\u2026</button>
          <span id="config-memory-status" class="config-note"></span>
        </div>
        <p class="config-note" data-i18n="config.memoryNote"></p>
      </div>
      <div id="config-oauth" class="config-env" hidden>
        <h3 data-i18n="config.oauthTitle">OAuth (device flow)</h3>
        <div id="config-oauth-rows" class="config-env-rows"></div>
        <p class="config-note" data-i18n="config.oauthNote"></p>
      </div>
      <div id="config-pool" class="config-env" hidden>
        <h3 data-i18n="config.poolTitle">Credential pool</h3>
        <div id="config-pool-rows" class="config-env-rows"></div>
        <div class="config-add">
          <span class="config-add-label" data-i18n="config.poolAddLabel">Add pool key</span>
          <input id="config-pool-add-provider" type="text" placeholder="provider (openai, anthropic, …)" />
          <input id="config-pool-add-key" type="password" placeholder="API key" />
          <input id="config-pool-add-label" type="text" placeholder="label (optional)" />
          <button id="config-pool-add-btn" class="ghost" data-i18n="config.add">Add</button>
        </div>
        <p class="config-note" data-i18n="config.poolNote"></p>
      </div>
      <div id="config-messaging" class="config-env" hidden>
        <h3 data-i18n="config.messagingTitle">Messaging platforms</h3>
        <div id="config-messaging-rows" class="config-env-rows"></div>
        <p class="config-note" data-i18n="config.messagingNote"></p>
      </div>
      <div id="config-update" class="config-env" hidden>
        <h3 data-i18n="config.updateTitle">Software update</h3>
        <div id="config-update-rows" class="config-env-rows"></div>
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
    this.root.querySelector("#config-env-validate-btn")!.addEventListener("click", () => {
      this.validateEnvKey().catch(() => undefined);
    });
    this.root.querySelector("#config-env-add-btn")!.addEventListener("click", () => {
      this.addEnvKey().catch(() => undefined);
    });
    this.root.querySelector("#config-memory-reset-btn")!.addEventListener("click", () => {
      this.resetMemory().catch(() => undefined);
    });
    this.root.querySelector("#config-pool-add-btn")!.addEventListener("click", () => {
      this.addPoolEntry().catch(() => undefined);
    });
    this.root.querySelector("#config-raw")!.addEventListener("click", () => {
      this.openRaw().catch(() => undefined);
    });
    this.root.querySelector("#config-raw-cancel")!.addEventListener("click", () => {
      (this.root.querySelector("#config-raw-dialog") as HTMLDialogElement).close();
    });
    this.root.querySelector("#config-raw-save")!.addEventListener("click", () => {
      this.saveRaw().catch(() => undefined);
    });
    // P501: filter config keys by path or value substring.
    this.root.querySelector("#config-filter")!.addEventListener("input", () => {
      this.renderRows();
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
      this.renderEnv().catch(() => undefined);
      this.renderMemory().catch(() => undefined);
      this.renderPool().catch(() => undefined);
      this.renderOAuth().catch(() => undefined);
      this.renderSchema().catch(() => undefined);
      this.renderMessaging().catch(() => undefined);
      this.renderTts().catch(() => undefined);
      this.renderUpdate().catch(() => undefined);
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
    // P501: optional key/value substring filter.
    const filter = (this.root.querySelector("#config-filter") as HTMLInputElement | null)?.value.trim().toLowerCase() ?? "";
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
      if (filter) {
        const value = this.base.get(path) ?? "";
        if (!path.toLowerCase().includes(filter) && !value.toLowerCase().includes(filter)) continue;
      }
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

  /** Env section (P320): /api/env posture rows (file vs process env),
   * per-key delete, and an add form writing through PUT /api/env. */
  private async renderEnv(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-env") as HTMLElement;
    const rows = this.root.querySelector("#config-env-rows") as HTMLElement;
    rows.innerHTML = "";
    block.hidden = false;
    let vars: { key: string; in_file: boolean; in_process_env: boolean }[];
    if (client) {
      try {
        const payload = await client.envList();
        vars = payload.vars;
      } catch {
        vars = this.envKeys.map((key) => ({ key, in_file: true, in_process_env: false }));
      }
    } else {
      vars = this.envKeys.map((key) => ({ key, in_file: true, in_process_env: false }));
    }
    if (!vars.length) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.config.envEmpty;
      rows.appendChild(empty);
      return;
    }
    for (const variable of vars) {
      const row = document.createElement("div");
      row.className = "config-env-row";
      const keyEl = document.createElement("span");
      keyEl.className = "config-env-chip";
      keyEl.textContent = variable.key;
      row.appendChild(keyEl);
      const source = document.createElement("span");
      source.className = "jobs-counts";
      source.textContent = variable.in_file
        ? variable.in_process_env
          ? t.config.envBoth
          : t.config.envFile
        : t.config.envProcess;
      row.appendChild(source);
      const reveal = document.createElement("button");
      reveal.className = "ghost";
      reveal.textContent = "\u{1F441}";
      reveal.title = t.config.envRevealTitle;
      reveal.onclick = () => this.toggleEnvReveal(variable.key, row);
      row.appendChild(reveal);
      if (variable.in_file) {
        const remove = document.createElement("button");
        remove.className = "ghost danger";
        remove.textContent = "\u{1F5D1}";
        remove.title = t.config.envRemoveTitle;
        remove.onclick = () => this.removeEnvKey(variable.key);
        row.appendChild(remove);
      }
      rows.appendChild(row);
    }
  }

  /** Show/hide one env value inline via POST /api/env/reveal (P336). */
  private async toggleEnvReveal(key: string, row: HTMLElement): Promise<void> {
    const existing = row.querySelector(".config-env-reveal");
    if (existing) {
      existing.remove();
      return;
    }
    const client = this.client();
    if (!client) return;
    try {
      const value = await client.envReveal(key);
      const shown = document.createElement("code");
      shown.className = "config-env-reveal";
      shown.textContent = value;
      row.appendChild(shown);
    } catch (error) {
      this.status(
        t.config.envFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** Config schema/defaults reference over /api/config/schema (P336). */
  private async renderSchema(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-schema") as HTMLElement;
    const rows = this.root.querySelector("#config-schema-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    try {
      const fields = await client.configSchema();
      if (!fields.length) {
        block.hidden = true;
        return;
      }
      block.hidden = false;
      for (const field of fields) {
        const row = document.createElement("div");
        row.className = "config-env-row";
        const keyEl = document.createElement("span");
        keyEl.className = "config-env-chip";
        keyEl.textContent = field.path;
        row.appendChild(keyEl);
        const meta = document.createElement("span");
        meta.className = "jobs-counts";
        meta.textContent = `${field.type} \u00b7 ${JSON.stringify(field.default)}`;
        row.appendChild(meta);
        rows.appendChild(row);
      }
    } catch {
      block.hidden = true;
    }
  }

  /** TTS section (P345): provider posture, ElevenLabs voice picker
   * over GET /api/audio/elevenlabs/voices, save via PUT /api/config,
   * and a preview button over POST /api/audio/speak. */
  /** Software update panel (P353): probe the git checkout for pending
   * updates and apply them in place over /api/update/check + POST
   * /api/update. */
  private async renderUpdate(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-update") as HTMLElement;
    const rows = this.root.querySelector("#config-update-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    block.hidden = false;

    const makeRow = (label: string, value: string): HTMLElement => {
      const row = document.createElement("div");
      row.className = "config-env-row";
      const chip = document.createElement("span");
      chip.className = "config-env-chip";
      chip.textContent = `${label}: ${value}`;
      row.appendChild(chip);
      return row;
    };

    let check: UpdateCheckResult;
    try {
      check = await client.updateCheck();
    } catch (error) {
      rows.appendChild(
        makeRow(
          t.config.updateStatus,
          error instanceof Error ? error.message : String(error),
        ),
      );
      return;
    }

    rows.appendChild(makeRow(t.config.updateVersion, check.current_version));
    rows.appendChild(makeRow(t.config.updateMethod, check.install_method));
    let stateText: string;
    if (check.error) {
      stateText = check.error;
    } else if (!check.update_available) {
      stateText = t.config.updateUpToDate;
    } else if (check.behind === -1) {
      stateText = t.config.updateShallow;
    } else {
      stateText = t.config.updateBehind.replace("{count}", String(check.behind ?? "?"));
    }
    rows.appendChild(makeRow(t.config.updateStatus, stateText));
    if (check.update_command) {
      rows.appendChild(makeRow(t.config.updateCommand, check.update_command));
    }

    const actions = document.createElement("div");
    actions.className = "config-env-row";
    const checkBtn = document.createElement("button");
    checkBtn.className = "ghost";
    checkBtn.textContent = t.config.updateCheck;
    checkBtn.onclick = () => {
      this.renderUpdate().catch(() => undefined);
    };
    actions.appendChild(checkBtn);

    if (check.can_apply && check.update_available) {
      const applyBtn = document.createElement("button");
      applyBtn.className = "primary";
      applyBtn.textContent = t.config.updateApply;
      applyBtn.onclick = async () => {
        applyBtn.disabled = true;
        applyBtn.textContent = t.config.updateApplying;
        try {
          const result = await client.updateApply();
          this.status(
            t.config.updateApplied
              .replace("{sha}", (result.new_sha || "").slice(0, 8))
              .replace("{count}", String(result.new_commits ?? 0)),
          );
          await this.renderUpdate();
        } catch (error) {
          this.status(
            t.config.updateFailed.replace(
              "{error}",
              error instanceof Error ? error.message : String(error),
            ),
            true,
          );
          applyBtn.disabled = false;
          applyBtn.textContent = t.config.updateApply;
        }
      };
      actions.appendChild(applyBtn);
    }
    rows.appendChild(actions);
  }

  private async renderTts(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-tts") as HTMLElement;
    const rows = this.root.querySelector("#config-tts-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    const provider = String(this.base.get("tts.provider") ?? "openai");
    block.hidden = false;

    const providerRow = document.createElement("div");
    providerRow.className = "config-env-row";
    const providerChip = document.createElement("span");
    providerChip.className = "config-env-chip";
    providerChip.textContent = `tts.provider = ${provider}`;
    providerRow.appendChild(providerChip);

    const preview = document.createElement("button");
    preview.className = "ghost";
    preview.textContent = `\u{1F50A} ${t.config.ttsPreview}`;
    preview.onclick = async () => {
      preview.disabled = true;
      try {
        const dataUrl = await client.audioSpeak(t.config.ttsSample);
        const audio = new Audio(dataUrl);
        await audio.play();
      } catch (error) {
        this.status(
          t.config.ttsPreviewFailed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          ),
          true,
        );
      } finally {
        preview.disabled = false;
      }
    };
    providerRow.appendChild(preview);
    rows.appendChild(providerRow);

    if (provider === "edge") {
      // P349: free Microsoft neural voices — a plain voice-name input
      // (no API key, no catalog endpoint to feed a picker).
      const edgeRow = document.createElement("div");
      edgeRow.className = "config-env-row";
      const edgeLabel = document.createElement("span");
      edgeLabel.className = "config-env-chip";
      edgeLabel.textContent = "tts.edge.voice";
      edgeRow.appendChild(edgeLabel);
      const input = document.createElement("input");
      input.type = "text";
      const current = String(this.base.get("tts.edge.voice") ?? "en-US-AriaNeural");
      input.value = current;
      input.placeholder = "en-US-AriaNeural";
      edgeRow.appendChild(input);
      const save = document.createElement("button");
      save.className = "ghost";
      save.textContent = t.config.save;
      save.disabled = true;
      input.oninput = () => {
        save.disabled = input.value.trim() === current || input.value.trim() === "";
      };
      save.onclick = async () => {
        save.disabled = true;
        try {
          await client.configSave({ "tts.edge.voice": input.value.trim() }, []);
          this.status(t.config.saved.replace("{count}", "1"));
          await this.refresh();
        } catch (error) {
          this.status(
            t.config.saveFailed.replace(
              "{error}",
              error instanceof Error ? error.message : String(error),
            ),
            true,
          );
        }
      };
      edgeRow.appendChild(save);
      rows.appendChild(edgeRow);
      return;
    }

    if (provider !== "elevenlabs") return;

    const voiceRow = document.createElement("div");
    voiceRow.className = "config-env-row";
    const voiceLabel = document.createElement("span");
    voiceLabel.className = "config-env-chip";
    voiceLabel.textContent = "tts.elevenlabs.voice_id";
    voiceRow.appendChild(voiceLabel);
    try {
      const result = await client.elevenlabsVoices();
      if (!result.available) {
        const note = document.createElement("span");
        note.className = "jobs-counts";
        note.textContent = result.error === "unauthorized"
          ? t.config.ttsVoicesUnauthorized
          : t.config.ttsVoicesUnavailable;
        voiceRow.appendChild(note);
        rows.appendChild(voiceRow);
        return;
      }
      const select = document.createElement("select");
      const current = String(this.base.get("tts.elevenlabs.voice_id") ?? "");
      for (const voice of result.voices) {
        const option = document.createElement("option");
        option.value = voice.voice_id;
        option.textContent = voice.label;
        if (voice.voice_id === current) option.selected = true;
        select.appendChild(option);
      }
      voiceRow.appendChild(select);
      const save = document.createElement("button");
      save.className = "ghost";
      save.textContent = t.config.save;
      save.disabled = true;
      select.onchange = () => {
        save.disabled = select.value === current;
      };
      save.onclick = async () => {
        save.disabled = true;
        try {
          await client.configSave({ "tts.elevenlabs.voice_id": select.value }, []);
          this.status(t.config.saved.replace("{count}", "1"));
          await this.refresh();
        } catch (error) {
          this.status(
            t.config.saveFailed.replace(
              "{error}",
              error instanceof Error ? error.message : String(error),
            ),
            true,
          );
        }
      };
      voiceRow.appendChild(save);
    } catch {
      const note = document.createElement("span");
      note.className = "jobs-counts";
      note.textContent = t.config.ttsVoicesUnavailable;
      voiceRow.appendChild(note);
    }
    rows.appendChild(voiceRow);
  }

  /** Messaging-platform posture + enable toggle + test probe over
   * /api/messaging/platforms (hermes ChannelsPage parity; P341). */
  private async renderMessaging(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-messaging") as HTMLElement;
    const rows = this.root.querySelector("#config-messaging-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    let platforms;
    try {
      platforms = await client.messagingPlatforms();
    } catch {
      block.hidden = true;
      return;
    }
    if (!platforms.length) {
      block.hidden = true;
      return;
    }
    block.hidden = false;
    for (const platform of platforms) {
      const row = document.createElement("div");
      row.className = "config-env-row";
      const chip = document.createElement("span");
      chip.className = "config-env-chip";
      chip.textContent = platform.name;
      chip.title = platform.description;
      row.appendChild(chip);
      const state = document.createElement("span");
      state.className = "jobs-counts";
      const missingEnv = platform.env_vars.filter((envVar) => envVar.required && !envVar.is_set);
      state.textContent = platform.enabled
        ? missingEnv.length
          ? `${platform.state} \u00b7 ${missingEnv.map((envVar) => envVar.key).join(", ")}`
          : platform.state
        : platform.state;
      row.appendChild(state);
      const toggle = document.createElement("button");
      toggle.className = "ghost";
      toggle.textContent = platform.enabled
        ? t.config.messagingDisable
        : t.config.messagingEnable;
      toggle.onclick = async () => {
        toggle.disabled = true;
        try {
          await client.messagingPlatformUpdate(platform.id, { enabled: !platform.enabled });
          this.status(t.config.restartNote);
          await this.renderMessaging();
        } catch (error) {
          this.status(
            t.config.messagingFailed.replace(
              "{error}",
              error instanceof Error ? error.message : String(error),
            ),
            true,
          );
        } finally {
          toggle.disabled = false;
        }
      };
      row.appendChild(toggle);
      if (platform.enabled) {
        const test = document.createElement("button");
        test.className = "ghost";
        test.textContent = t.config.messagingTest;
        test.onclick = async () => {
          test.disabled = true;
          try {
            const result = await client.messagingPlatformTest(platform.id);
            state.textContent = result.message;
          } catch (error) {
            state.textContent = error instanceof Error ? error.message : String(error);
          } finally {
            test.disabled = false;
          }
        };
        row.appendChild(test);
      }
      rows.appendChild(row);
      // P342: env-credential inputs for platforms whose adapters honor
      // env keys (telegram/discord/slack) — save/clear ride the P337
      // PUT /api/messaging/platforms/:id surface.
      if (platform.env_vars.length) {
        const envRow = document.createElement("div");
        envRow.className = "config-env-row";
        const spacer = document.createElement("span");
        spacer.className = "jobs-counts";
        spacer.textContent = "\u00a0";
        envRow.appendChild(spacer);
        for (const envVar of platform.env_vars) {
          const input = document.createElement("input");
          input.type = "password";
          input.placeholder = envVar.is_set
            ? envVar.redacted_value || envVar.key
            : envVar.key;
          input.title = envVar.key;
          envRow.appendChild(input);
          const save = document.createElement("button");
          save.className = "ghost";
          save.textContent = "\u{1F4BE}";
          save.title = `${t.config.messagingSaveEnv} ${envVar.key}`;
          save.onclick = async () => {
            const value = input.value.trim();
            if (!value) return;
            save.disabled = true;
            try {
              await client.messagingPlatformUpdate(platform.id, {
                env: { [envVar.key]: value },
              });
              input.value = "";
              this.status(t.config.restartNote);
              await this.renderMessaging();
            } catch (error) {
              this.status(
                t.config.messagingFailed.replace(
                  "{error}",
                  error instanceof Error ? error.message : String(error),
                ),
                true,
              );
            } finally {
              save.disabled = false;
            }
          };
          envRow.appendChild(save);
          if (envVar.is_set) {
            const clear = document.createElement("button");
            clear.className = "ghost danger";
            clear.textContent = "\u{1F5D1}";
            clear.title = `${t.config.messagingClearEnv} ${envVar.key}`;
            clear.onclick = async () => {
              clear.disabled = true;
              try {
                await client.messagingPlatformUpdate(platform.id, {
                  clear_env: [envVar.key],
                });
                this.status(t.config.restartNote);
                await this.renderMessaging();
              } catch (error) {
                this.status(
                  t.config.messagingFailed.replace(
                    "{error}",
                    error instanceof Error ? error.message : String(error),
                  ),
                  true,
                );
              } finally {
                clear.disabled = false;
              }
            };
            envRow.appendChild(clear);
          }
        }
        rows.appendChild(envRow);
      }
    }
  }

  /** Persistent-memory census + reset over /api/memory (P323). */
  private async renderMemory(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-memory") as HTMLElement;
    const rows = this.root.querySelector("#config-memory-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    try {
      const status = await client.memoryStatus();
      block.hidden = false;
      block.title = status.dir;
      for (const file of status.files) {
        const row = document.createElement("div");
        row.className = "config-env-row";
        const keyEl = document.createElement("span");
        keyEl.className = "config-env-chip";
        keyEl.textContent = file.file;
        row.appendChild(keyEl);
        const meta = document.createElement("span");
        meta.className = "jobs-counts";
        if (file.exists) {
          const limit = file.file === "MEMORY.md" ? status.char_limits.memory : status.char_limits.user;
          meta.textContent = `${file.desc} \u00b7 ${file.entries} ${t.config.memoryEntries} \u00b7 ${file.bytes} B \u00b7 ${t.config.memoryLimit} ${limit}`;
        } else {
          meta.textContent = t.config.memoryMissing;
        }
        row.appendChild(meta);
        rows.appendChild(row);
      }
    } catch {
      block.hidden = true;
    }
  }

  private async resetMemory(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const target = (this.root.querySelector("#config-memory-target") as HTMLSelectElement)
      .value as "all" | "memory" | "user";
    if (!window.confirm(t.config.memoryResetConfirm)) return;
    const note = this.root.querySelector("#config-memory-status") as HTMLElement;
    try {
      const deleted = await client.memoryReset(target);
      note.textContent = deleted.length
        ? t.config.memoryResetDone.replace("{files}", deleted.join(", "))
        : t.config.memoryResetNone;
      await this.renderMemory();
    } catch (error) {
      note.textContent = t.config.memoryResetFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** OAuth device-flow posture over /api/oauth/status (P334). */
  private async renderOAuth(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-oauth") as HTMLElement;
    const rows = this.root.querySelector("#config-oauth-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    try {
      const status = await client.oauthStatus();
      block.hidden = false;
      const row = document.createElement("div");
      row.className = "config-env-row";
      const chip = document.createElement("span");
      chip.className = "config-env-chip";
      chip.textContent = status.logged_in
        ? `${t.config.oauthLoggedIn}${status.expired ? " (expired)" : ""}`
        : t.config.oauthLoggedOut;
      row.appendChild(chip);
      const meta = document.createElement("span");
      meta.className = "jobs-counts";
      const bits: string[] = [`provider: ${status.provider}`];
      if (status.logged_in) {
        bits.push(status.token_preview);
        if (status.scopes) bits.push(status.scopes);
        if (status.expires_at > 0) bits.push(new Date(status.expires_at * 1000).toLocaleString());
      }
      meta.textContent = bits.join(" · ");
      row.appendChild(meta);
      if (status.portal_url) {
        const link = document.createElement("a");
        link.href = status.portal_url;
        link.target = "_blank";
        link.rel = "noreferrer";
        link.textContent = t.config.oauthPortal;
        row.appendChild(link);
      }
      rows.appendChild(row);
      await this.renderProviderOAuth(rows);
    } catch {
      block.hidden = true;
    }
  }

  /** P350: provider OAuth handshake rows (start/poll/disconnect) over
   * /api/providers/oauth* — the desktop twin of hermes' provider OAuth
   * settings page (lean: single device-code provider). */
  private async renderProviderOAuth(rows: HTMLElement): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (this.oauthPollTimer !== null) {
      window.clearTimeout(this.oauthPollTimer);
      this.oauthPollTimer = null;
    }
    let providers: ProviderOAuthRow[] = [];
    try {
      ({ providers } = await client.providersOAuth());
    } catch {
      return;
    }
    for (const provider of providers) {
      const row = document.createElement("div");
      row.className = "config-env-row";
      const chip = document.createElement("span");
      chip.className = "config-env-chip";
      chip.textContent = provider.name;
      row.appendChild(chip);
      const meta = document.createElement("span");
      meta.className = "jobs-counts";
      const bits = [provider.flow];
      if (provider.status.logged_in) {
        bits.push(provider.status.token_preview);
        if (provider.status.expires_at > 0) {
          bits.push(new Date(provider.status.expires_at * 1000).toLocaleString());
        }
      } else if (!provider.configured) {
        bits.push(t.config.oauthProviderNotConfigured);
      }
      meta.textContent = bits.join(" \u00b7 ");
      row.appendChild(meta);
      if (provider.status.logged_in) {
        const disconnect = document.createElement("button");
        disconnect.className = "ghost";
        disconnect.textContent = t.config.oauthProviderDisconnect;
        disconnect.onclick = async () => {
          if (!window.confirm(t.config.oauthProviderDisconnectConfirm)) return;
          disconnect.disabled = true;
          try {
            await client.providersOAuthDisconnect(provider.id);
            this.status(t.config.oauthProviderDisconnected);
            await this.refresh();
          } catch (error) {
            this.status(
              t.config.oauthProviderFailed.replace(
                "{error}",
                error instanceof Error ? error.message : String(error),
              ),
              true,
            );
            disconnect.disabled = false;
          }
        };
        row.appendChild(disconnect);
      } else if (provider.configured) {
        const connect = document.createElement("button");
        connect.className = "ghost";
        connect.textContent = t.config.oauthProviderConnect;
        connect.onclick = () => {
          connect.disabled = true;
          void this.startProviderOAuth(client, provider, row);
        };
        row.appendChild(connect);
      }
      rows.appendChild(row);
    }
  }

  /** P350: run one device-flow session — start, show the user code,
   * poll until complete/error, then refresh the posture. */
  private async startProviderOAuth(
    client: GatewayClient,
    provider: ProviderOAuthRow,
    row: HTMLElement,
  ): Promise<void> {
    const pending = document.createElement("span");
    pending.className = "jobs-counts";
    try {
      const start = await client.providersOAuthStart(provider.id);
      pending.textContent = t.config.oauthProviderPending.replace("{code}", start.user_code || "\u2014");
      row.appendChild(pending);
      if (start.verification_uri) {
        const link = document.createElement("a");
        link.href = start.verification_uri;
        link.target = "_blank";
        link.rel = "noreferrer";
        link.textContent = t.config.oauthProviderOpen;
        row.appendChild(link);
      }
      const pollIntervalMs = Math.max(start.interval || 5, 3) * 1000;
      const poll = async (): Promise<void> => {
        try {
          const result = await client.providersOAuthPoll(provider.id, start.session_id);
          if (result.status === "complete") {
            this.status(t.config.oauthProviderComplete);
            await this.refresh();
            return;
          }
          if (result.status === "error") {
            this.status(
              t.config.oauthProviderFailed.replace("{error}", result.error || "unknown"),
              true,
            );
            await this.refresh();
            return;
          }
        } catch {
          // Session cancelled or gateway restarted — stop polling quietly.
          await this.refresh();
          return;
        }
        this.oauthPollTimer = window.setTimeout(() => void poll(), pollIntervalMs);
      };
      this.oauthPollTimer = window.setTimeout(() => void poll(), pollIntervalMs);
    } catch (error) {
      this.status(
        t.config.oauthProviderFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
      await this.refresh();
    }
  }

  private async renderPool(): Promise<void> {
    const client = this.client();
    const block = this.root.querySelector("#config-pool") as HTMLElement;
    const rows = this.root.querySelector("#config-pool-rows") as HTMLElement;
    rows.innerHTML = "";
    if (!client) {
      block.hidden = true;
      return;
    }
    try {
      const payload = await client.credentialsPool();
      block.hidden = false;
      if (payload.providers.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = t.config.poolEmpty;
        rows.appendChild(empty);
        return;
      }
      for (const provider of payload.providers) {
        for (const entry of provider.entries) {
          const row = document.createElement("div");
          row.className = "config-env-row";
          const keyEl = document.createElement("span");
          keyEl.className = "config-env-chip";
          keyEl.textContent = provider.provider;
          row.appendChild(keyEl);
          const meta = document.createElement("span");
          meta.className = "jobs-counts";
          meta.textContent = `${entry.label} · ${entry.token_preview} · ${entry.source} · #${entry.request_count}`;
          row.appendChild(meta);
          const remove = document.createElement("button");
          remove.className = "ghost danger";
          remove.textContent = "✕";
          remove.title = t.session.removeAttachment;
          remove.onclick = () => {
            this.removePoolEntry(provider.provider, entry.index).catch(() => undefined);
          };
          row.appendChild(remove);
          rows.appendChild(row);
        }
      }
    } catch {
      block.hidden = true;
    }
  }

  private async addPoolEntry(): Promise<void> {
    const client = this.client();
    const providerEl = this.root.querySelector("#config-pool-add-provider") as HTMLInputElement;
    const keyEl = this.root.querySelector("#config-pool-add-key") as HTMLInputElement;
    const labelEl = this.root.querySelector("#config-pool-add-label") as HTMLInputElement;
    const provider = providerEl.value.trim();
    const apiKey = keyEl.value.trim();
    if (!client || !provider || !apiKey) {
      (provider ? keyEl : providerEl).focus();
      return;
    }
    try {
      await client.credentialsPoolAdd(provider, apiKey, labelEl.value.trim());
      providerEl.value = "";
      keyEl.value = "";
      labelEl.value = "";
      await this.renderPool();
      this.status(t.config.poolSaved);
    } catch (error) {
      this.status(
        t.config.poolFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async removePoolEntry(provider: string, index: number): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.config.poolRemoveConfirm)) return;
    try {
      await client.credentialsPoolRemove(provider, index);
      await this.renderPool();
      this.status(t.config.poolSaved);
    } catch (error) {
      this.status(
        t.config.poolFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** Live-probe the pending key/value before saving (P608). */
  private async validateEnvKey(): Promise<boolean> {
    const client = this.client();
    const keyEl = this.root.querySelector("#config-env-add-key") as HTMLInputElement;
    const valueEl = this.root.querySelector("#config-env-add-value") as HTMLInputElement;
    const key = keyEl.value.trim();
    const value = valueEl.value.trim();
    if (!client || !key || !value) {
      this.status(t.config.envValidateNeed, true);
      return false;
    }
    try {
      const result = await client.providersValidate(key, value);
      if (result.ok) {
        this.status(
          result.models && result.models.length
            ? t.config.envValidateOkModels.replace("{count}", String(result.models.length))
            : t.config.envValidateOk,
        );
        return true;
      }
      this.status(
        result.reachable
          ? t.config.envValidateBad.replace("{message}", result.message)
          : t.config.envValidateUnreachable,
        result.reachable,
      );
      return !result.reachable;
    } catch (error) {
      this.status(
        t.config.envFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
      return false;
    }
  }

  private async addEnvKey(): Promise<void> {
    const client = this.client();
    const keyEl = this.root.querySelector("#config-env-add-key") as HTMLInputElement;
    const valueEl = this.root.querySelector("#config-env-add-value") as HTMLInputElement;
    const key = keyEl.value.trim();
    if (!client || !key) {
      keyEl.focus();
      return;
    }
    const known = ["OPENAI_API_KEY", "OPENROUTER_API_KEY", "XAI_API_KEY", "GEMINI_API_KEY", "OPENAI_BASE_URL"];
    if (known.includes(key) && valueEl.value.trim()) {
      const ok = await this.validateEnvKey();
      if (!ok) return;
    }
    try {
      await client.envSet(key, valueEl.value);
      keyEl.value = "";
      valueEl.value = "";
      await this.renderEnv();
      this.status(t.config.envSaved);
    } catch (error) {
      this.status(
        t.config.envFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async removeEnvKey(key: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.config.envRemoveConfirm.replace("{key}", key))) return;
    try {
      await client.envDelete(key);
      await this.renderEnv();
      this.status(t.config.envSaved);
    } catch (error) {
      this.status(
        t.config.envFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
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
