// Model visibility dialog (P252) — dependency-free port of hermes
// apps/desktop `components/model-visibility-dialog.tsx`: search box,
// per-provider collapse + master checkbox (with indeterminate), one
// toggle row per collapsed model family, and an add-provider footer
// (wired to the onboarding provider guidance — the shell has no
// dedicated providers settings page). Edits persist through the
// model-visibility store and re-render the model picker live.

import type { GatewayClient, ModelOptionsPayload, ModelOptionRow } from "./gateway";
import { fmt, t } from "./i18n";
import {
  collapseModelFamilies,
  effectiveVisibleKeys,
  getCollapsedProviders,
  getVisibleModels,
  modelVisibilityKey,
  setProviderVisibility,
  setVisibleModels,
  toggleCollapsedProvider,
  toggleModelVisibility,
} from "./model-visibility";

export interface VisibilityHooks {
  /** Reload model options and re-render the model picker. */
  onChanged(): void;
  /** Open the provider-setup guidance surface. */
  onAddProvider(): void;
}

function normalize(query: string): string {
  return query.trim().toLowerCase();
}

export class ModelVisibilityDialog {
  private dialog: HTMLDialogElement;
  private body: HTMLDivElement;
  private search: HTMLInputElement;
  private payload: ModelOptionsPayload | null = null;

  constructor(
    private client: () => GatewayClient | null,
    private hooks: VisibilityHooks,
  ) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "model-visibility-dialog";
    const header = document.createElement("div");
    header.className = "model-visibility-header";
    const title = document.createElement("h2");
    title.textContent = t.picker.visibilityTitle;
    header.appendChild(title);
    const searchWrap = document.createElement("div");
    searchWrap.className = "model-visibility-search";
    this.search = document.createElement("input");
    this.search.type = "text";
    this.search.placeholder = t.picker.visibilitySearch;
    this.search.addEventListener("input", () => this.renderBody());
    searchWrap.appendChild(this.search);
    this.body = document.createElement("div");
    this.body.className = "model-visibility-body";
    const footer = document.createElement("div");
    footer.className = "model-visibility-footer";
    const reset = document.createElement("button");
    reset.className = "ghost";
    reset.type = "button";
    reset.textContent = t.picker.resetVisibility;
    reset.onclick = () => {
      setVisibleModels(null);
      this.renderBody();
      this.hooks.onChanged();
    };
    const addProvider = document.createElement("button");
    addProvider.className = "ghost";
    addProvider.type = "button";
    addProvider.textContent = t.picker.addProvider;
    addProvider.onclick = () => {
      this.dialog.close();
      this.hooks.onAddProvider();
    };
    footer.append(reset, addProvider);
    this.dialog.append(header, searchWrap, this.body, footer);
    document.body.appendChild(this.dialog);
  }

  async open(): Promise<void> {
    this.search.value = "";
    this.body.innerHTML = "";
    const loading = document.createElement("div");
    loading.className = "model-picker-loading";
    loading.textContent = t.picker.loading;
    this.body.appendChild(loading);
    this.dialog.showModal();

    const client = this.client();
    if (!client) {
      loading.textContent = t.picker.notConnected;
      return;
    }
    try {
      this.payload = await client.modelOptions();
      this.renderBody();
    } catch (error) {
      loading.textContent = fmt(t.picker.loadFailed, { error });
    }
  }

  private providers(): ModelOptionRow[] {
    return (this.payload?.providers ?? []).filter(
      (provider) => (provider.models ?? []).length > 0,
    );
  }

  private renderBody(): void {
    this.body.innerHTML = "";
    const providers = this.providers();
    if (!providers.length) {
      const empty = document.createElement("div");
      empty.className = "model-picker-loading";
      empty.textContent = t.picker.visibilityEmpty;
      this.body.appendChild(empty);
      return;
    }
    const stored = getVisibleModels();
    const visible = effectiveVisibleKeys(stored, providers);
    const collapsed = getCollapsedProviders();
    const q = normalize(this.search.value);

    for (const provider of providers) {
      const families = collapseModelFamilies(provider.models ?? []);
      const matches = (familyId: string) =>
        !q ||
        `${familyId} ${provider.name ?? ""} ${provider.slug}`.toLowerCase().includes(q);
      const shown = families.filter((family) => matches(family.id));
      if (!shown.length) continue;

      const section = document.createElement("div");
      section.className = "mv-provider";

      const head = document.createElement("div");
      head.className = "mv-provider-head";
      const label = document.createElement("button");
      label.type = "button";
      label.className = "mv-provider-label";
      const caret = document.createElement("span");
      caret.className = "mv-caret";
      const isCollapsed = collapsed.includes(provider.slug) && !q;
      caret.textContent = isCollapsed ? "▸" : "▾";
      const name = document.createElement("span");
      name.className = "mv-provider-name";
      name.textContent = provider.name || provider.slug;
      label.append(caret, name);
      label.onclick = () => {
        toggleCollapsedProvider(provider.slug);
        this.renderBody();
      };
      const master = document.createElement("input");
      master.type = "checkbox";
      master.className = "mv-master";
      const onCount = families.filter((family) =>
        visible.has(modelVisibilityKey(provider.slug, family.id)),
      ).length;
      master.checked = onCount > 0;
      master.indeterminate = onCount > 0 && onCount < families.length;
      master.title = t.picker.visibilityTitle;
      master.onchange = () => {
        setVisibleModels(
          setProviderVisibility(getVisibleModels(), providers, provider.slug, master.checked),
        );
        this.renderBody();
        this.hooks.onChanged();
      };
      head.append(label, master);
      section.appendChild(head);

      if (!isCollapsed) {
        for (const family of shown) {
          const key = modelVisibilityKey(provider.slug, family.id);
          const row = document.createElement("label");
          row.className = "mv-row";
          const box = document.createElement("input");
          box.type = "checkbox";
          box.checked = visible.has(key);
          box.onchange = () => {
            setVisibleModels(
              toggleModelVisibility(getVisibleModels(), providers, provider.slug, family.id),
            );
            this.renderBody();
            this.hooks.onChanged();
          };
          const label_ = document.createElement("span");
          label_.className = "mv-model";
          label_.textContent = family.id;
          row.append(box, label_);
          if (family.fastId) {
            const fast = document.createElement("span");
            fast.className = "mv-fast";
            fast.textContent = "+fast";
            fast.title = family.fastId;
            row.appendChild(fast);
          }
          section.appendChild(row);
        }
      }
      this.body.appendChild(section);
    }

  }
}
