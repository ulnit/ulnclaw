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
      <div id="models-view-body" class="models-view-body"></div>
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

    (this.root.querySelector("#models-view-count") as HTMLElement).textContent =
      v.count.replace("{providers}", String(payload.providers.length));
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
          <th>${escapeHtml(v.colCaps)}</th><th class="num">${escapeHtml(v.colPrice)}</th>
        </tr></thead><tbody>${rows}</tbody></table>` : `<p class="empty">${escapeHtml(v.noModels)}</p>`}
      </section>`;
  }
}
