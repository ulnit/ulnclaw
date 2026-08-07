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
  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="models-view-header">
        <span id="models-view-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="models-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="models-view-status" class="config-status" hidden></div>
      <div id="models-view-gateway" class="models-view-gateway"></div>
      <div id="models-view-body" class="models-view-body"></div>
      <h3 class="config-section" data-i18n="modelsView.usageTitle">Model usage (30 days)</h3>
      <div id="models-view-usage"></div>
    `;
    this.root.querySelector("#models-view-refresh")!.addEventListener("click", () => {
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
      this.render(payload);
      this.renderGatewayModel().catch(() => undefined);
      this.renderUsage().catch(() => undefined);
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

  private render(payload: ModelOptionsPayload): void {
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

    for (const provider of payload.providers) {
      parts.push(this.renderProvider(provider, payload.model));
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
      box.innerHTML = `
        <section class="models-view-provider">
          <h3 class="config-section">${escapeHtml(v.gatewayTitle)}</h3>
          <div class="monitoring-row">
            <span class="monitoring-label"><code>${escapeHtml(info.provider)}/${escapeHtml(info.model)}</code></span>
            <span class="monitoring-value">${escapeHtml(info.base_url)}</span>
            <span class="jobs-counts">${info.context.effective ? `${escapeHtml(v.gatewayContext)}: ${fmtNum(info.context.effective)}` : ""} ${icons}</span>
          </div>
        </section>`;
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
          <td>${featured.has(model) ? "★ " : ""}${escapeHtml(model)}</td>
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
