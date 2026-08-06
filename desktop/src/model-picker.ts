// Desktop model picker — GUI twin of hermes' model-picker overlay
// (apps/desktop/src/components/model-picker.tsx): per-session model
// lock over GET /api/model/options + POST /api/sessions/:id/model.
// Dependency-free <dialog> like the hatch/settings overlays.

import type { GatewayClient, ModelOptionsPayload, ModelOptionRow } from "./gateway";

export interface ModelSelection {
  model: string;
  provider?: string;
}

export class ModelPickerOverlay {
  private dialog: HTMLDialogElement;
  private body: HTMLDivElement;

  constructor(
    private client: () => GatewayClient | null,
    private sessionId: () => string | null,
    /** Current session lock (null = gateway default). */
    private currentModel: () => string | null,
    private onLocked: (selection: ModelSelection) => void = () => {},
  ) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "model-picker-dialog";
    const header = document.createElement("div");
    header.className = "model-picker-header";
    const title = document.createElement("h2");
    title.textContent = "🧠 Model for this session";
    header.appendChild(title);
    this.body = document.createElement("div");
    this.body.className = "model-picker-body";
    this.dialog.append(header, this.body);
    document.body.appendChild(this.dialog);
  }

  async open(): Promise<void> {
    this.body.innerHTML = "";
    const loading = document.createElement("div");
    loading.className = "model-picker-loading";
    loading.textContent = "Loading model inventory…";
    this.body.appendChild(loading);
    this.dialog.showModal();

    const client = this.client();
    if (!client) {
      loading.textContent = "Gateway not connected.";
      return;
    }
    try {
      const payload = await client.modelOptions();
      this.render(payload);
    } catch (err) {
      loading.textContent = `Failed to load models: ${err}`;
    }
  }

  private render(payload: ModelOptionsPayload): void {
    this.body.innerHTML = "";
    const sessionLocked = this.currentModel();

    // Gateway default row — resets the session lock to the configured model.
    const defaultRow = document.createElement("div");
    defaultRow.className = "model-picker-option";
    const isDefaultActive = !sessionLocked || sessionLocked === payload.model;
    if (isDefaultActive) defaultRow.classList.add("active");
    defaultRow.innerHTML = "";
    const defLabel = document.createElement("div");
    defLabel.className = "model-picker-option-name";
    defLabel.textContent = `${payload.model} (gateway default)`;
    const defMeta = document.createElement("div");
    defMeta.className = "model-picker-option-meta";
    defMeta.textContent = payload.provider;
    defaultRow.append(defLabel, defMeta);
    defaultRow.onclick = () => this.pick(payload.model, payload.provider);
    this.body.appendChild(defaultRow);

    if (sessionLocked && sessionLocked !== payload.model) {
      const lockNote = document.createElement("div");
      lockNote.className = "model-picker-locknote";
      lockNote.textContent = `🔒 This session is locked to ${sessionLocked}.`;
      this.body.appendChild(lockNote);
    }

    for (const provider of payload.providers || []) {
      this.body.appendChild(this.renderProvider(provider, payload, sessionLocked));
    }

    if (!(payload.providers || []).length) {
      const empty = document.createElement("div");
      empty.className = "model-picker-loading";
      empty.textContent = "No providers reported by the gateway.";
      this.body.appendChild(empty);
    }
  }

  private renderProvider(
    provider: ModelOptionRow,
    payload: ModelOptionsPayload,
    sessionLocked: string | null,
  ): HTMLDivElement {
    const wrap = document.createElement("div");
    wrap.className = "model-picker-provider";

    const head = document.createElement("div");
    head.className = "model-picker-provider-head";
    const name = provider.name || provider.slug;
    const bits: string[] = [name];
    if (provider.current) bits.push("current");
    if (provider.authenticated === false) bits.push("⚠ not authenticated");
    head.textContent = bits.join("  ·  ");
    wrap.appendChild(head);

    const models = provider.models && provider.models.length ? provider.models : [];
    if (!models.length) {
      const hint = document.createElement("div");
      hint.className = "model-picker-option-meta";
      hint.textContent = provider.base_url
        ? `No models listed${provider.base_url ? ` — ${provider.base_url}` : ""}`
        : "No models listed";
      wrap.appendChild(hint);
      return wrap;
    }
    for (const model of models) {
      const option = document.createElement("div");
      option.className = "model-picker-option model-row";
      const active = sessionLocked ? model === sessionLocked : model === payload.model && !!provider.current;
      if (active) option.classList.add("active");
      const label = document.createElement("div");
      label.className = "model-picker-option-name";
      label.textContent = model;
      option.appendChild(label);
      if (provider.authenticated === false) {
        option.classList.add("disabled");
        option.title = "Provider not authenticated";
      } else {
        option.onclick = () => this.pick(model, provider.slug);
      }
      wrap.appendChild(option);
    }
    return wrap;
  }

  private async pick(model: string, provider?: string): Promise<void> {
    const client = this.client();
    const sessionId = this.sessionId();
    if (!client || !sessionId) return;
    try {
      await client.lockSessionModel(sessionId, model, provider);
      this.onLocked({ model, provider });
      this.dialog.close();
    } catch (err) {
      const note = document.createElement("div");
      note.className = "model-picker-locknote";
      note.textContent = `Lock failed: ${err}`;
      this.body.prepend(note);
    }
  }
}
