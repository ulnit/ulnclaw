// Models view — read-only provider/model inventory over
// `/api/model/options` (same payload as the chat model picker): provider
// cards with auth posture, catalog freshness, API/doc links and a
// per-model table with family, context window, capability icons and
// pricing. Selection stays in the chat model picker.

import type { GatewayClient, ModelOptionRow, ModelOptionsPayload } from "./gateway";
import { t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fmtNum(n: number | null | undefined): string {
  if (!n || n <= 0) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

export class ModelsViewWidget {
  /** P516: last inventory so the live filter re-renders without refetching. */
  private payload: ModelOptionsPayload | null = null;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="models-view-header">
        <span id="models-view-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <input id="models-view-filter" type="search" data-i18n-ph="modelsView.filterPlaceholder" />
        <button id="models-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="models-view-status" class="config-status" hidden></div>
      <div id="models-view-gateway" class="models-view-gateway"></div>
      <div id="models-view-body" class="models-view-body"></div>
      <h3 class="config-section" data-i18n="modelsView.auxTitle">Auxiliary task models</h3>
      <div id="models-view-aux"></div>
      <h3 class="config-section" data-i18n="modelsView.moaTitle">Mixture-of-Agents presets</h3>
      <div id="models-view-moa"></div>
      <h3 class="config-section" data-i18n="modelsView.usageTitle">Model usage (30 days)</h3>
      <div id="models-view-usage"></div>
      <h3 class="config-section" data-i18n="modelsView.endpointsTitle">Custom endpoints</h3>
      <div id="models-view-endpoints"></div>
      <div class="config-add">
        <input id="models-view-ep-id" type="text" placeholder="id (mylab)" />
        <input id="models-view-ep-url" type="text" placeholder="https://api.example.com/v1" />
        <input id="models-view-ep-model" type="text" placeholder="default model" />
        <select id="models-view-ep-mode"><option value="openai">openai</option><option value="anthropic">anthropic</option></select>
        <input id="models-view-ep-key" type="password" placeholder="API key (stored in .env)" />
      </div>
      <div class="config-add">
        <button id="models-view-ep-test" class="ghost" data-i18n="modelsView.endpointsTest">Test</button>
        <button id="models-view-ep-add" class="ghost" data-i18n="config.add">Add</button>
        <span id="models-view-ep-status" class="config-note"></span>
      </div>
    `;
    this.root.querySelector("#models-view-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#models-view-filter")!.addEventListener("input", () => {
      if (this.payload) this.render(this.payload, this.filterQuery());
    });
    this.root.querySelector("#models-view-body")!.addEventListener("click", (event) => {
      const target = (event.target as HTMLElement).closest(".models-view-model") as HTMLElement | null;
      if (!target || !target.dataset.model) return;
      const model = target.dataset.model;
      void navigator.clipboard.writeText(model).then(
        () => this.status(t.modelsView.copiedModel.replace("{model}", model)),
        () => this.status(t.modelsView.copyFailed, true),
      );
    });
    this.root.querySelector("#models-view-ep-test")!.addEventListener("click", () => {
      this.validateEndpoint().catch(() => undefined);
    });
    this.root.querySelector("#models-view-ep-add")!.addEventListener("click", () => {
      this.addEndpoint().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* on-demand only */
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#models-view-status") as HTMLElement;
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
      const payload = await client.modelOptions();
      this.render(payload, this.filterQuery());
      this.renderGatewayModel().catch(() => undefined);
      this.renderAuxiliary().catch(() => undefined);
      this.renderMoa().catch(() => undefined);
      this.renderUsage().catch(() => undefined);
      this.renderEndpoints().catch(() => undefined);
      this.status("");
    } catch (error) {
      this.status(
        t.modelsView.loadFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** P516: current live-filter query (lowercased, trimmed). */
  private filterQuery(): string {
    const input = this.root.querySelector("#models-view-filter") as HTMLInputElement | null;
    return (input?.value || "").trim().toLowerCase();
  }

  private render(payload: ModelOptionsPayload, query = ""): void {
    this.payload = payload;
    const body = this.root.querySelector("#models-view-body") as HTMLElement;
    const v = t.modelsView;
    const parts: string[] = [];

    parts.push(`<p class="models-view-current">${escapeHtml(v.current)}: <code>${escapeHtml(payload.provider)}/${escapeHtml(payload.model)}</code>` +
      (payload.catalog_cache
        ? ` · ${escapeHtml(v.catalog)}: ${payload.catalog_cache.providers} ${escapeHtml(v.providersLower)}${payload.catalog_cache.fresh ? "" : ` (${escapeHtml(v.stale)})`}`
        : "") +
      `</p>`);

    if (payload.providers.length === 0) {
      parts.push(`<p class="empty">${escapeHtml(v.none)}</p>`);
      body.innerHTML = parts.join("");
      return;
    }

    let rendered = 0;
    for (const provider of payload.providers) {
      const filtered = this.filterProvider(provider, query);
      if (!filtered) continue;
      parts.push(this.renderProvider(filtered, payload.model));
      rendered += 1;
    }
    if (rendered === 0) {
      parts.push(`<p class="empty">${escapeHtml(v.filterNoMatch)}</p>`);
    }
    body.innerHTML = parts.join("");
    body.querySelectorAll<HTMLButtonElement>(".models-view-set").forEach((btn) => {
      btn.onclick = () => {
        this.setModel(btn.dataset.provider || "", btn.dataset.model || "").catch(() => undefined);
      };
    });

    (this.root.querySelector("#models-view-count") as HTMLElement).textContent =
      v.count.replace("{providers}", String(payload.providers.length));
  }

  /** P516: narrow a provider's model rows by the live filter; a provider
   * whose name/slug matches keeps all its models. Returns null when the
   * provider has nothing to show. */
  private filterProvider(provider: ModelOptionRow, query: string): ModelOptionRow | null {
    if (!query) return provider;
    const name = `${provider.name || ""} ${provider.slug}`.toLowerCase();
    if (name.includes(query)) return provider;
    const models = (provider.models || []).filter((model) => {
      const family = provider.capabilities?.[model]?.family || "";
      return `${model} ${family}`.toLowerCase().includes(query);
    });
    if (models.length === 0) return null;
    return { ...provider, models };
  }

  /** Custom endpoint rows over /api/providers/custom-endpoints (P333). */
  private async renderEndpoints(): Promise<void> {
    const client = this.client();
    const box = this.root.querySelector("#models-view-endpoints") as HTMLElement;
    if (!client) {
      box.innerHTML = "";
      return;
    }
    const v = t.modelsView;
    try {
      const payload = await client.customEndpoints();
      if (payload.endpoints.length === 0) {
        box.innerHTML = `<p class="config-note">${escapeHtml(v.endpointsEmpty)}</p>`;
        return;
      }
      box.innerHTML = payload.endpoints
        .map((endpoint) => `
          <div class="monitoring-row">
            <span class="monitoring-label"><code>${escapeHtml(endpoint.id)}</code> <span class="models-view-badge">${escapeHtml(endpoint.mode)}</span> <span class="models-view-badge ${endpoint.key_state === "missing" ? "warn" : "ok"}">${escapeHtml(endpoint.key_state)}</span></span>
            <span class="monitoring-value">${escapeHtml(endpoint.base_url)}${endpoint.model ? ` · ${escapeHtml(endpoint.model)}` : ""}</span>
            <span class="jobs-counts">
              <button class="ghost models-view-ep-activate" data-id="${escapeHtml(endpoint.id)}">${escapeHtml(v.endpointsActivate)}</button>
              <button class="ghost danger models-view-ep-delete" data-id="${escapeHtml(endpoint.id)}">✕</button>
            </span>
          </div>`)
        .join("");
      box.querySelectorAll<HTMLButtonElement>(".models-view-ep-activate").forEach((btn) => {
        btn.onclick = () => {
          this.activateEndpoint(btn.dataset.id || "").catch(() => undefined);
        };
      });
      box.querySelectorAll<HTMLButtonElement>(".models-view-ep-delete").forEach((btn) => {
        btn.onclick = () => {
          this.deleteEndpoint(btn.dataset.id || "").catch(() => undefined);
        };
      });
    } catch {
      box.innerHTML = "";
    }
  }

  private endpointStatus(message: string): void {
    const el = this.root.querySelector("#models-view-ep-status") as HTMLElement;
    el.textContent = message;
  }

  /** Probe the drafted endpoint's /models URL (P333). */
  private async validateEndpoint(): Promise<void> {
    const client = this.client();
    const url = (this.root.querySelector("#models-view-ep-url") as HTMLInputElement).value.trim();
    const key = (this.root.querySelector("#models-view-ep-key") as HTMLInputElement).value;
    if (!client) return;
    try {
      const result = await client.customEndpointsValidate({ base_url: url, api_key: key || undefined });
      this.endpointStatus(
        result.ok
          ? `${result.models.length} models`
          : result.message,
      );
    } catch (error) {
      this.endpointStatus(error instanceof Error ? error.message : String(error));
    }
  }

  /** Upsert the drafted endpoint (P333). */
  private async addEndpoint(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const id = (this.root.querySelector("#models-view-ep-id") as HTMLInputElement).value.trim();
    const baseUrl = (this.root.querySelector("#models-view-ep-url") as HTMLInputElement).value.trim();
    const model = (this.root.querySelector("#models-view-ep-model") as HTMLInputElement).value.trim();
    const mode = (this.root.querySelector("#models-view-ep-mode") as HTMLSelectElement).value;
    const key = (this.root.querySelector("#models-view-ep-key") as HTMLInputElement).value;
    if (!id || !baseUrl) {
      this.endpointStatus("id + base_url are required");
      return;
    }
    try {
      await client.customEndpointsUpsert({
        id,
        base_url: baseUrl,
        model: model || undefined,
        mode,
        api_key: key || undefined,
      });
      (this.root.querySelector("#models-view-ep-id") as HTMLInputElement).value = "";
      (this.root.querySelector("#models-view-ep-url") as HTMLInputElement).value = "";
      (this.root.querySelector("#models-view-ep-model") as HTMLInputElement).value = "";
      (this.root.querySelector("#models-view-ep-key") as HTMLInputElement).value = "";
      this.endpointStatus(t.modelsView.endpointsSaved);
      await this.renderEndpoints();
    } catch (error) {
      this.endpointStatus(
        t.modelsView.endpointsFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private async activateEndpoint(id: string): Promise<void> {
    const client = this.client();
    if (!client || !id) return;
    try {
      await client.customEndpointsActivate(id);
      this.status(t.modelsView.endpointsActivated);
      await this.renderGatewayModel();
    } catch (error) {
      this.status(
        t.modelsView.endpointsFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async deleteEndpoint(id: string): Promise<void> {
    const client = this.client();
    if (!client || !id) return;
    if (!window.confirm(t.modelsView.endpointsDeleteConfirm.replace("{id}", id))) return;
    try {
      await client.customEndpointsDelete(id);
      await this.renderEndpoints();
    } catch (error) {
      this.status(
        t.modelsView.endpointsFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** Gateway model card over /api/model/info (P332). */
  private async renderGatewayModel(): Promise<void> {
    const client = this.client();
    const box = this.root.querySelector("#models-view-gateway") as HTMLElement;
    if (!client) {
      box.innerHTML = "";
      return;
    }
    const v = t.modelsView;
    try {
      const info = await client.modelInfo();
      const caps = info.capabilities;
      const icons = caps
        ? [caps.reasoning ? "\u{1F9E0}" : "", caps.tools ? "\u{1F527}" : "", caps.vision ? "\u{1F441}" : ""]
            .filter(Boolean)
            .join(" ")
        : "";
      let recommended = "";
      try {
        const rec = await client.modelRecommendedDefault(info.provider);
        if (rec.model && rec.model !== info.model) recommended = rec.model;
      } catch {
        /* catalog offline — skip the recommendation row */
      }
      box.innerHTML = `
        <section class="models-view-provider">
          <h3 class="config-section">${escapeHtml(v.gatewayTitle)}</h3>
          <div class="monitoring-row">
            <span class="monitoring-label"><code>${escapeHtml(info.provider)}/${escapeHtml(info.model)}</code></span>
            <span class="monitoring-value">${escapeHtml(info.base_url)}</span>
            <span class="jobs-counts">${info.context.effective ? `${escapeHtml(v.gatewayContext)}: ${fmtNum(info.context.effective)}` : ""} ${icons}</span>
          </div>
          ${recommended ? `
          <div class="monitoring-row">
            <span class="monitoring-label">${escapeHtml(v.recommendedDefault)}: <code>${escapeHtml(recommended)}</code></span>
            <button id="models-view-rec-set" class="ghost">${escapeHtml(v.gatewaySet)}</button>
          </div>` : ""}
        </section>`;
      if (recommended) {
        box.querySelector("#models-view-rec-set")?.addEventListener("click", () => {
          void this.setModel(info.provider, recommended);
        });
      }
    } catch {
      box.innerHTML = "";
    }
  }

  /** Persist a new gateway model over POST /api/model/set (P332). */
  private async setModel(provider: string, model: string): Promise<void> {
    const client = this.client();
    if (!client || !provider || !model) return;
    const v = t.modelsView;
    const message = v.gatewaySetConfirm
      .replace("{provider}", provider)
      .replace("{model}", model);
    if (!window.confirm(message)) return;
    try {
      await client.modelSet(provider, model);
      this.status(v.gatewaySetDone);
      await this.renderGatewayModel();
    } catch (error) {
      this.status(
        v.gatewaySetFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  /** Per-task auxiliary model assignments (GET /api/model/auxiliary;
   * hermes auxiliary slot UI parity; P605). */
  private async renderAuxiliary(): Promise<void> {
    const client = this.client();
    const box = this.root.querySelector("#models-view-aux") as HTMLElement;
    if (!client) {
      box.innerHTML = "";
      return;
    }
    const v = t.modelsView;
    try {
      const payload = await client.modelAuxiliary();
      box.innerHTML = "";
      for (const task of payload.tasks) {
        const row = document.createElement("div");
        row.className = "monitoring-row models-view-aux-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.innerHTML = `<code>${escapeHtml(task.task)}</code>`;
        const current = document.createElement("span");
        current.className = "monitoring-value";
        const providerPinned = task.provider !== "auto" && task.provider !== "";
        const modelPinned = task.model !== "auto" && task.model !== "";
        if (providerPinned || modelPinned) {
          const shownProvider = providerPinned ? task.provider : payload.main.provider;
          const shownModel = modelPinned ? task.model : payload.main.model;
          current.innerHTML = `<code>${escapeHtml(shownProvider)}/${escapeHtml(shownModel)}</code>`;
        } else {
          current.textContent = v.auxInherit;
        }
        row.append(label, current);
        const providerInput = document.createElement("input");
        providerInput.type = "text";
        providerInput.placeholder = v.auxProviderPh;
        providerInput.value = providerPinned ? task.provider : "";
        const modelInput = document.createElement("input");
        modelInput.type = "text";
        modelInput.placeholder = v.auxModelPh;
        modelInput.value = modelPinned ? task.model : "";
        const saveBtn = document.createElement("button");
        saveBtn.className = "ghost";
        saveBtn.textContent = v.auxSave;
        saveBtn.addEventListener("click", () => {
          void this.setAuxiliary(task.task, providerInput.value.trim(), modelInput.value.trim());
        });
        const resetBtn = document.createElement("button");
        resetBtn.className = "ghost";
        resetBtn.textContent = v.auxReset;
        resetBtn.addEventListener("click", () => {
          void this.setAuxiliary(task.task, "", "");
        });
        row.append(providerInput, modelInput, saveBtn, resetBtn);
        box.appendChild(row);
      }
    } catch {
      box.innerHTML = "";
    }
  }

  /** Pin or reset one auxiliary task slot (POST /api/model/set; P605). */
  private async setAuxiliary(task: string, provider: string, model: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    const v = t.modelsView;
    try {
      const result = await client.modelSetAuxiliary(task, provider, model);
      this.status(result.reset ? v.auxResetDone.replace("{task}", task) : v.auxSaved.replace("{task}", task));
      await this.renderAuxiliary();
    } catch (error) {
      this.status(
        v.auxFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    }
  }

  /** MoA preset summary + JSON editor (GET/PUT /api/model/moa; hermes
   * MoA dashboard parity, lean; P606). */
  private async renderMoa(): Promise<void> {
    const client = this.client();
    const box = this.root.querySelector("#models-view-moa") as HTMLElement;
    if (!client) {
      box.innerHTML = "";
      return;
    }
    const v = t.modelsView;
    try {
      const payload = await client.modelMoa();
      box.innerHTML = "";
      const names = Object.keys(payload.presets || {});
      if (names.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = v.moaEmpty;
        box.appendChild(empty);
      }
      for (const name of names.sort()) {
        const preset = payload.presets[name];
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = name === payload.default_preset ? `${name} ★` : name;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        const refs = (preset.reference_models || [])
          .map((slot) => `${slot.provider}:${slot.model}${slot.enabled === false ? " (off)" : ""}`)
          .join(", ");
        value.textContent = `${v.moaRefs}: ${refs} — ${v.moaAggregator}: ${preset.aggregator?.provider}:${preset.aggregator?.model}`;
        value.title = value.textContent;
        row.append(label, value);
        box.appendChild(row);
      }
      const hint = document.createElement("p");
      hint.className = "config-note";
      hint.textContent = v.moaHint;
      box.appendChild(hint);
      const editor = document.createElement("textarea");
      editor.className = "models-view-moa-editor";
      editor.spellcheck = false;
      const editable = {
        default_preset: payload.default_preset ?? "",
        presets: payload.presets || {},
      };
      editor.value = JSON.stringify(editable, null, 2);
      box.appendChild(editor);
      const saveBtn = document.createElement("button");
      saveBtn.className = "ghost";
      saveBtn.textContent = v.moaSave;
      saveBtn.addEventListener("click", () => {
        void this.saveMoa(editor, saveBtn);
      });
      box.appendChild(saveBtn);
    } catch {
      box.innerHTML = "";
    }
  }

  /** Validate the editor JSON and PUT it (P606). */
  private async saveMoa(editor: HTMLTextAreaElement, button: HTMLButtonElement): Promise<void> {
    const client = this.client();
    if (!client) return;
    const v = t.modelsView;
    let parsed: unknown;
    try {
      parsed = JSON.parse(editor.value);
    } catch {
      this.status(v.moaBadJson, true);
      return;
    }
    button.disabled = true;
    try {
      await client.modelMoaSave(parsed);
      this.status(v.moaSaved);
      await this.renderMoa();
    } catch (error) {
      this.status(
        v.moaFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      button.disabled = false;
    }
  }

  /** Per-model usage table over /api/analytics/models (P328). */
  private async renderUsage(): Promise<void> {
    const client = this.client();
    const box = this.root.querySelector("#models-view-usage") as HTMLElement;
    if (!client) {
      box.innerHTML = "";
      return;
    }
    const v = t.modelsView;
    try {
      const payload = await client.analyticsModels(30);
      if (payload.models.length === 0) {
        box.innerHTML = `<p class="config-note">${escapeHtml(v.usageEmpty)}</p>`;
        return;
      }
      const rows = payload.models
        .map((row) => {
          const when = row.last_used_at
            ? new Date(row.last_used_at * 1000).toLocaleString()
            : "\u2014";
          return `
            <div class="monitoring-row">
              <span class="monitoring-label">${escapeHtml(row.model)}</span>
              <span class="monitoring-value">${row.sessions} ${escapeHtml(v.usageSessions)} \u00b7 ${row.messages} ${escapeHtml(v.usageMessages)} \u00b7 ${row.total_tokens.toLocaleString()} ${escapeHtml(v.usageTokens)}</span>
              <span class="jobs-counts">${escapeHtml(when)}</span>
            </div>`;
        })
        .join("");
      box.innerHTML = rows;
    } catch {
      box.innerHTML = "";
    }
  }

  private renderProvider(provider: ModelOptionRow, currentModel: string): string {
    const v = t.modelsView;
    const name = provider.name || provider.slug;
    const badges: string[] = [];
    if (provider.current) badges.push(`<span class="models-view-badge accent">${escapeHtml(v.currentBadge)}</span>`);
    badges.push(
      provider.authenticated
        ? `<span class="models-view-badge ok">${escapeHtml(v.authenticated)}</span>`
        : `<span class="models-view-badge warn">${escapeHtml(v.unauthenticated)}</span>`,
    );
    if (provider.catalog_stale) badges.push(`<span class="models-view-badge warn">${escapeHtml(v.stale)}</span>`);
    if (provider.key_env) badges.push(`<span class="models-view-badge">${escapeHtml(provider.key_env)}</span>`);

    const links: string[] = [];
    if (provider.api) links.push(`<a href="${escapeHtml(provider.api)}" target="_blank" rel="noreferrer">API</a>`);
    if (provider.doc) links.push(`<a href="${escapeHtml(provider.doc)}" target="_blank" rel="noreferrer">${escapeHtml(v.docs)}</a>`);

    const models = provider.models || [];
    const featured = new Set(provider.featured_models || []);
    const rows = models
      .map((model) => {
        const caps = provider.capabilities?.[model];
        const price = provider.pricing?.[model];
        const icons = [
          caps?.reasoning ? "🧠" : "",
          caps?.tools ? "🔧" : "",
          caps?.vision ? "👁" : "",
        ]
          .filter(Boolean)
          .join(" ");
        const isCurrent = provider.current && model === currentModel;
        return `<tr class="${isCurrent ? "models-view-current-row" : ""}">
          <td><span class="models-view-model" data-model="${escapeHtml(model)}" title="${escapeHtml(v.copyModelTitle)}">${featured.has(model) ? "★ " : ""}${escapeHtml(model)}</span></td>
          <td class="muted">${escapeHtml(caps?.family || "")}</td>
          <td class="num">${fmtNum(caps?.context_window)}</td>
          <td class="num">${fmtNum(caps?.max_output_tokens)}</td>
          <td>${icons}</td>
          <td class="num">${price ? `${escapeHtml(price.input)} / ${escapeHtml(price.output)}` : "—"}</td>
          <td>${isCurrent ? "" : `<button class="ghost models-view-set" data-provider="${escapeHtml(provider.slug)}" data-model="${escapeHtml(model)}" title="${escapeHtml(v.gatewaySet)}">⭢</button>`}</td>
        </tr>`;
      })
      .join("");

    return `
      <section class="models-view-provider">
        <h3 class="config-section">
          ${escapeHtml(name)} <span class="models-view-slug">${escapeHtml(provider.slug)}</span>
          ${badges.join(" ")}
          ${links.length ? `<span class="models-view-links">${links.join(" · ")}</span>` : ""}
        </h3>
        ${provider.base_url ? `<p class="models-view-baseurl">${escapeHtml(provider.base_url)}</p>` : ""}
        ${rows ? `<table class="usage-table"><thead><tr>
          <th>${escapeHtml(v.colModel)}</th><th>${escapeHtml(v.colFamily)}</th>
          <th class="num">${escapeHtml(v.colContext)}</th><th class="num">${escapeHtml(v.colMaxOut)}</th>
          <th>${escapeHtml(v.colCaps)}</th><th class="num">${escapeHtml(v.colPrice)}</th><th></th>
        </tr></thead><tbody>${rows}</tbody></table>` : `<p class="empty">${escapeHtml(v.noModels)}</p>`}
      </section>`;
  }
}
