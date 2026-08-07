// Pairing view — messaging-platform pairing management over
// `/api/pairing*` (hermes `pairing list/approve/revoke/clear-pending`
// parity): per-platform pending requests with one-click approve,
// approved grants with revoke, lockout badges and a clear-pending
// action.

import type { GatewayClient, PairingPlatform } from "./gateway";
import { t } from "./i18n";

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class PairingViewWidget {
  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="pairing-view-header">
        <span id="pairing-view-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <button id="pairing-view-clear" class="ghost" data-i18n="pairingView.clearPending">Clear pending</button>
        <button id="pairing-view-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
      </header>
      <div id="pairing-view-status" class="config-status" hidden></div>
      <div id="pairing-view-body"></div>
    `;
    this.root.querySelector("#pairing-view-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#pairing-view-clear")!.addEventListener("click", () => {
      this.clearAll().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* on-demand only */
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#pairing-view-status") as HTMLElement;
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
      const platforms = await client.pairingStatus();
      this.render(platforms);
      this.status("");
    } catch (error) {
      this.status(
        t.pairingView.loadFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private render(platforms: PairingPlatform[]): void {
    const body = this.root.querySelector("#pairing-view-body") as HTMLElement;
    const v = t.pairingView;
    const pendingTotal = platforms.reduce((sum, p) => sum + p.pending.length, 0);
    (this.root.querySelector("#pairing-view-count") as HTMLElement).textContent =
      v.count.replace("{platforms}", String(platforms.length)).replace("{pending}", String(pendingTotal));

    if (platforms.length === 0) {
      body.innerHTML = `<p class="empty">${escapeHtml(v.none)}</p>`;
      return;
    }

    body.innerHTML = platforms
      .map((platform) => {
        const lockBadge = platform.locked_out
          ? `<span class="models-view-badge warn">${escapeHtml(v.lockedOut)}</span>`
          : "";
        const pending = platform.pending.length
          ? `
            <h4 class="config-section">${escapeHtml(v.pendingTitle)}</h4>
            ${platform.pending
              .map(
                (request) => `
                  <div class="monitoring-row pairing-view-row">
                    <span class="monitoring-label">${escapeHtml(request.request_id)}</span>
                    <span class="monitoring-value">${escapeHtml(request.user_id)}${
                      request.user_name ? ` (${escapeHtml(request.user_name)})` : ""
                    } · ${v.age.replace("{minutes}", String(request.age_minutes))}</span>
                    <button class="ghost pairing-view-approve" data-platform="${escapeHtml(platform.platform)}" data-code="${escapeHtml(request.request_id)}">${escapeHtml(v.approve)}</button>
                  </div>`,
              )
              .join("")}`
          : "";
        const approved = platform.approved.length
          ? `
            <h4 class="config-section">${escapeHtml(v.approvedTitle)}</h4>
            ${platform.approved
              .map(
                (grant) => `
                  <div class="monitoring-row pairing-view-row">
                    <span class="monitoring-label">${escapeHtml(grant.user_id)}</span>
                    <span class="monitoring-value">${escapeHtml(grant.user_name)}</span>
                    <button class="ghost pairing-view-revoke" data-platform="${escapeHtml(platform.platform)}" data-user="${escapeHtml(grant.user_id)}">${escapeHtml(v.revoke)}</button>
                  </div>`,
              )
              .join("")}`
          : "";
        return `
          <section class="pairing-view-platform">
            <h3 class="config-section">${escapeHtml(platform.platform)} ${lockBadge}</h3>
            ${pending || approved ? pending + approved : `<p class="empty">${escapeHtml(v.emptyPlatform)}</p>`}
          </section>`;
      })
      .join("");

    for (const button of Array.from(body.querySelectorAll<HTMLButtonElement>(".pairing-view-approve"))) {
      button.addEventListener("click", () => {
        this.approve(button.dataset.platform || "", button.dataset.code || "").catch(() => undefined);
      });
    }
    for (const button of Array.from(body.querySelectorAll<HTMLButtonElement>(".pairing-view-revoke"))) {
      button.addEventListener("click", () => {
        this.revoke(button.dataset.platform || "", button.dataset.user || "").catch(() => undefined);
      });
    }
  }

  private async approve(platform: string, code: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      await client.pairingApprove(platform, code);
      this.status(t.pairingView.approvedNote.replace("{code}", code), false);
      await this.refresh();
    } catch (error) {
      this.status(
        t.pairingView.approveFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async revoke(platform: string, userId: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      await client.pairingRevoke(platform, userId);
      this.status(t.pairingView.revokedNote.replace("{user}", userId), false);
      await this.refresh();
    } catch (error) {
      this.status(
        t.pairingView.revokeFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }

  private async clearAll(): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      const cleared = await client.pairingClearPending();
      this.status(t.pairingView.clearedNote.replace("{count}", String(cleared)), false);
      await this.refresh();
    } catch (error) {
      this.status(
        t.pairingView.loadFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }
}
