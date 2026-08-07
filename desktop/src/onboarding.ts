// First-run onboarding overlay (P250) — scoped port of hermes
// apps/desktop `components/onboarding/`: welcome → provider setup →
// done, driven by GET /api/model/options (provider rows carry
// `authenticated` + `key_env` setup hints, hermes inventory parity).
// The shell has no config-write surface, so unconfigured providers
// render setup guidance (env var + config.toml snippet) with a
// re-check loop instead of a key form.

import type { GatewayClient, ModelOptionRow, ModelOptionsPayload } from "./gateway";
import { fmt, t } from "./i18n";

const STORAGE_KEY = "ul…boarded";

export class OnboardingOverlay {
  private dialog: HTMLDialogElement;
  private stepEl: HTMLElement;
  private bodyEl: HTMLElement;
  private step = 0;

  constructor(private client: () => GatewayClient | null) {
    this.dialog = document.createElement("dialog");
    this.dialog.id = "onboarding";
    this.dialog.innerHTML = `
      <div class="onb-glyph" aria-hidden="true">🦞</div>
      <h2 class="onb-title"></h2>
      <div class="onb-body"></div>
      <div class="onb-actions">
        <button class="ghost onb-skip" type="button"></button>
        <button class="primary onb-next" type="button"></button>
      </div>`;
    this.stepEl = this.dialog.querySelector(".onb-title")!;
    this.bodyEl = this.dialog.querySelector(".onb-body")!;
    const skip = this.dialog.querySelector<HTMLButtonElement>(".onb-skip")!;
    const next = this.dialog.querySelector<HTMLButtonElement>(".onb-next")!;
    skip.onclick = () => this.finish();
    next.onclick = () => this.advance();
    this.dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      this.finish();
    });
    document.body.appendChild(this.dialog);
    this.renderWelcome();
  }

  static hasCompleted(): boolean {
    try {
      return localStorage.getItem(STORAGE_KEY) === "1";
    } catch {
      return true;
    }
  }

  /** Open on first run only (unless forced from Settings). */
  async maybeOpen(force = false): Promise<void> {
    if (!force && OnboardingOverlay.hasCompleted()) return;
    this.step = 0;
    this.renderWelcome();
    this.dialog.showModal();
  }

  private finish(): void {
    try {
      localStorage.setItem(STORAGE_KEY, "1");
    } catch {
      // storage unavailable — onboarding will show again next run
    }
    this.dialog.close();
  }

  private advance(): void {
    if (this.step === 0) {
      this.step = 1;
      void this.renderProviders();
      return;
    }
    this.finish();
  }

  private renderWelcome(): void {
    this.stepEl.textContent = t.onboarding.welcomeTitle;
    this.bodyEl.innerHTML = "";
    const intro = document.createElement("p");
    intro.className = "onb-text";
    intro.textContent = t.onboarding.intro;
    const bullets = document.createElement("ul");
    bullets.className = "onb-list";
    for (const line of [t.onboarding.bullet1, t.onboarding.bullet2, t.onboarding.bullet3]) {
      const li = document.createElement("li");
      li.textContent = line;
      bullets.appendChild(li);
    }
    this.bodyEl.append(intro, bullets);
    this.setActions(t.onboarding.skip, t.onboarding.getStarted);
  }

  private async renderProviders(): Promise<void> {
    this.stepEl.textContent = t.onboarding.providersTitle;
    this.bodyEl.innerHTML = "";
    const loading = document.createElement("p");
    loading.className = "onb-text";
    loading.textContent = t.onboarding.loadingProviders;
    this.bodyEl.appendChild(loading);

    let payload: ModelOptionsPayload | null = null;
    try {
      payload = await this.client()?.modelOptions() ?? null;
    } catch {
      payload = null;
    }
    this.bodyEl.innerHTML = "";

    if (!payload) {
      const warning = document.createElement("p");
      warning.className = "onb-text";
      warning.textContent = t.onboarding.noInventory;
      this.bodyEl.appendChild(warning);
      this.setActions(t.onboarding.skip, t.onboarding.finish);
      return;
    }

    const current = document.createElement("p");
    current.className = "onb-text";
    current.textContent = fmt(t.onboarding.currentModel, { model: payload.model, provider: payload.provider });
    this.bodyEl.appendChild(current);

    const list = document.createElement("div");
    list.className = "onb-providers";
    const rows = [...payload.providers].sort((a, b) => {
      const authed = (r: ModelOptionRow) => (r.authenticated ? 0 : 1);
      return authed(a) - authed(b) || a.slug.localeCompare(b.slug);
    });
    for (const row of rows.slice(0, 12)) {
      list.appendChild(this.providerRow(row));
    }
    this.bodyEl.appendChild(list);

    const recheck = document.createElement("button");
    recheck.className = "ghost onb-recheck";
    recheck.type = "button";
    recheck.textContent = t.onboarding.recheck;
    recheck.onclick = () => void this.renderProviders();
    this.bodyEl.appendChild(recheck);
    this.setActions(t.onboarding.skip, t.onboarding.finish);
  }

  private providerRow(row: ModelOptionRow): HTMLElement {
    const node = document.createElement("div");
    node.className = "onb-provider" + (row.authenticated ? " onb-ok" : "");
    const name = document.createElement("div");
    name.className = "onb-provider-name";
    name.textContent = row.name || row.slug;
    const status = document.createElement("div");
    status.className = "onb-provider-status";
    if (row.authenticated) {
      status.textContent = row.current ? t.onboarding.active : t.onboarding.configured;
    } else if (row.key_env) {
      status.textContent = fmt(t.onboarding.needsEnv, { env: row.key_env });
    } else {
      status.textContent = t.onboarding.notConfigured;
    }
    node.append(name, status);
    if (!row.authenticated && row.key_env) {
      node.title = fmt(t.onboarding.needsEnvTitle, { env: row.key_env });
    }
    return node;
  }

  private setActions(skipLabel: string, nextLabel: string): void {
    this.dialog.querySelector<HTMLButtonElement>(".onb-skip")!.textContent = skipLabel;
    this.dialog.querySelector<HTMLButtonElement>(".onb-next")!.textContent = nextLabel;
  }
}
