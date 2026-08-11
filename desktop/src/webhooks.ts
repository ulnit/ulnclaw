import { ICON } from "./icons";
// Webhooks view — manages dynamic webhook subscriptions (hermes
// `ulnclaw webhook subscribe|list|remove|test`) over the gateway
// `/api/webhooks/subscriptions` API: list with URL copy + signed test
// fire + delete, and a create/update form.

import type { GatewayClient, WebhookSubscription } from "./gateway";
import { t } from "./i18n";

const DELIVER_TARGETS = [
  "log", "telegram", "discord", "slack", "feishu", "dingtalk", "matrix",
  "mattermost", "teams", "googlechat", "line", "irc", "email", "whatsapp",
];

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class WebhooksWidget {
  private busy = false;
  /** P517: cached subscriptions so the live filter re-renders without refetching. */
  private allSubs: WebhookSubscription[] = [];

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="webhooks-header">
        <span id="webhooks-count" class="jobs-counts"></span>
        <span class="spacer"></span>
        <input id="webhooks-filter" type="search" data-i18n-ph="webhooks.filterPlaceholder" />
        <button id="webhooks-refresh" class="ghost icon-btn" title="Refresh" data-i18n-title="kanban.refresh">${ICON.rotate}</button>
      </header>
      <div id="webhooks-status" class="config-status" hidden></div>
      <div id="webhooks-list" class="webhooks-list"></div>
      <details id="webhooks-form-wrap" class="webhooks-form-wrap" open>
        <summary data-i18n="webhooks.createTitle">New subscription</summary>
        <form id="webhooks-form" class="webhooks-form">
          <label><span data-i18n="webhooks.name">Name</span>
            <input name="name" type="text" required pattern="[a-z0-9 _-]+" data-i18n-ph="webhooks.namePh" placeholder="build-events" />
          </label>
          <label><span data-i18n="webhooks.description">Description</span>
            <input name="description" type="text" data-i18n-ph="webhooks.descriptionPh" placeholder="CI notifications" />
          </label>
          <label><span data-i18n="webhooks.events">Events (comma-separated, empty = all)</span>
            <input name="events" type="text" data-i18n-ph="webhooks.eventsPh" placeholder="push, ci" />
          </label>
          <label><span data-i18n="webhooks.deliver">Deliver target</span>
            <input name="deliver" type="text" list="webhooks-deliver-targets" value="log" />
            <datalist id="webhooks-deliver-targets">
              ${DELIVER_TARGETS.map((target) => `<option value="${target}"></option>`).join("")}
            </datalist>
          </label>
          <label><span data-i18n="webhooks.deliverChat">Deliver chat id (optional)</span>
            <input name="deliver_chat_id" type="text" />
          </label>
          <label class="check"><input name="deliver_only" type="checkbox" />
            <span data-i18n="webhooks.deliverOnly">Direct delivery (no agent, zero LLM cost)</span>
          </label>
          <label><span data-i18n="webhooks.prompt">Prompt / message</span>
            <textarea name="prompt" rows="2" data-i18n-ph="webhooks.promptPh" placeholder="Summarize this event…"></textarea>
          </label>
          <label><span data-i18n="webhooks.skills">Skills (comma-separated)</span>
            <input name="skills" type="text" />
          </label>
          <label><span data-i18n="webhooks.script">Script (optional)</span>
            <input name="script" type="text" data-i18n-ph="webhooks.scriptPh" placeholder="./handle.sh" />
          </label>
          <label><span data-i18n="webhooks.secret">Secret (blank = auto-mint)</span>
            <input name="secret" type="text" autocomplete="off" />
          </label>
          <button type="submit" class="primary" data-i18n="webhooks.create">Create</button>
        </form>
      </details>
    `;
    this.root.querySelector("#webhooks-refresh")!.addEventListener("click", () => {
      this.refresh().catch(() => undefined);
    });
    this.root.querySelector("#webhooks-filter")!.addEventListener("input", () => {
      this.applyFilter();
    });
    (this.root.querySelector("#webhooks-form") as HTMLFormElement).addEventListener("submit", (event) => {
      event.preventDefault();
      this.create().catch(() => undefined);
    });
  }

  start(): void {
    this.refresh().catch(() => undefined);
  }

  stop(): void {
    /* on-demand only */
  }

  private status(message: string, isError = false): void {
    const el = this.root.querySelector("#webhooks-status") as HTMLElement;
    el.hidden = !message;
    el.textContent = message;
    el.classList.toggle("error", isError);
  }

  async refresh(): Promise<void> {
    const client = this.client();
    const list = this.root.querySelector("#webhooks-list") as HTMLElement;
    if (!client) {
      this.status(t.config.notConnected, true);
      return;
    }
    try {
      const payload = await client.webhooksList();
      this.allSubs = payload.subscriptions;
      this.applyFilter();
      const count = this.root.querySelector("#webhooks-count") as HTMLElement;
      count.textContent = t.webhooks.count.replace("{count}", String(payload.subscriptions.length));
      this.status("");
    } catch (error) {
      list.innerHTML = "";
      this.status(
        t.webhooks.loadFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    }
  }

  /** P517: re-render the cached subscriptions through the live filter. */
  private applyFilter(): void {
    const input = this.root.querySelector("#webhooks-filter") as HTMLInputElement | null;
    const query = (input?.value || "").trim().toLowerCase();
    const subs = query
      ? this.allSubs.filter((sub) =>
          `${sub.name} ${sub.description || ""} ${sub.url} ${sub.deliver} ${sub.events.join(" ")}`
            .toLowerCase()
            .includes(query),
        )
      : this.allSubs;
    this.render(subs, Boolean(query));
  }

  private render(subs: WebhookSubscription[], filtered = false): void {
    const list = this.root.querySelector("#webhooks-list") as HTMLElement;
    list.innerHTML = "";
    if (subs.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = filtered ? t.webhooks.filterNoMatch : t.webhooks.empty;
      list.appendChild(empty);
      return;
    }
    for (const sub of subs) {
      const card = document.createElement("div");
      card.className = "webhook-card";
      const events = sub.events.length > 0 ? sub.events.join(", ") : t.webhooks.allEvents;
      card.innerHTML = `
        <div class="webhook-head">
          <span class="webhook-name">${escapeHtml(sub.name)}</span>
          <span class="webhook-deliver">${escapeHtml(sub.deliver)}${sub.deliver_only ? " · " + escapeHtml(t.webhooks.direct) : ""}</span>
          <span class="spacer"></span>
          <button class="ghost webhook-test" data-i18n="webhooks.test">Test</button>
          <button class="ghost webhook-copy" data-i18n="webhooks.copy">Copy URL</button>
          <button class="ghost webhook-duplicate" data-i18n="webhooks.duplicate">Duplicate</button>
          <button class="ghost danger webhook-delete" data-i18n="webhooks.delete">Delete</button>
        </div>
        ${sub.description ? `<div class="webhook-desc">${escapeHtml(sub.description)}</div>` : ""}
        <div class="webhook-url"><code>${escapeHtml(sub.url)}</code></div>
        <div class="webhook-meta">
          <span>${escapeHtml(t.webhooks.events)}: ${escapeHtml(events)}</span>
          ${sub.script ? `<span>script: ${escapeHtml(sub.script)}</span>` : ""}
          ${sub.has_secret ? `<span>secret: ${escapeHtml(sub.secret_preview)}</span>` : ""}
        </div>
      `;
      card.querySelector(".webhook-test")!.addEventListener("click", () => {
        this.test(sub.name).catch(() => undefined);
      });
      card.querySelector(".webhook-copy")!.addEventListener("click", () => {
        void navigator.clipboard.writeText(sub.url).then(
          () => this.status(t.webhooks.copied),
          () => this.status(t.webhooks.copyFailed, true),
        );
      });
      card.querySelector(".webhook-delete")!.addEventListener("click", () => {
        this.remove(sub.name).catch(() => undefined);
      });
      card.querySelector(".webhook-duplicate")!.addEventListener("click", () => {
        this.prefillFrom(sub);
      });
      list.appendChild(card);
    }
  }

  private formValues(): Record<string, string> {
    const form = this.root.querySelector("#webhooks-form") as HTMLFormElement;
    const data = new FormData(form);
    const values: Record<string, string> = {};
    for (const [key, value] of data.entries()) {
      values[key] = String(value).trim();
    }
    values.deliver_only = (form.elements.namedItem("deliver_only") as HTMLInputElement).checked
      ? "true"
      : "";
    return values;
  }

  private resetForm(): void {
    (this.root.querySelector("#webhooks-form") as HTMLFormElement).reset();
    (this.root.querySelector("#webhooks-form [name=deliver]") as HTMLInputElement).value = "log";
  }

  /** P578: prefill the create form from an existing subscription (⧉ duplicate). */
  private prefillFrom(sub: WebhookSubscription): void {
    const form = this.root.querySelector("#webhooks-form") as HTMLFormElement;
    form.reset();
    const set = (name: string, value: string): void => {
      (form.elements.namedItem(name) as HTMLInputElement | HTMLTextAreaElement).value = value;
    };
    set("name", `${sub.name}-copy`);
    set("description", sub.description);
    set("events", sub.events.join(", "));
    set("deliver", sub.deliver);
    set("script", sub.script ?? "");
    (form.elements.namedItem("deliver_only") as HTMLInputElement).checked = sub.deliver_only;
    const wrap = this.root.querySelector("#webhooks-form-wrap") as HTMLDetailsElement;
    wrap.open = true;
    wrap.scrollIntoView({ block: "nearest" });
    (form.elements.namedItem("name") as HTMLInputElement).focus();
    this.status(t.webhooks.duplicatePrefilled.replace("{name}", sub.name));
  }

  private async create(): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    const values = this.formValues();
    if (!values.name) return;
    this.busy = true;
    try {
      const reply = await client.webhooksCreate({
        name: values.name,
        description: values.description || undefined,
        events: values.events || undefined,
        prompt: values.prompt || undefined,
        skills: values.skills || undefined,
        deliver: values.deliver || undefined,
        deliver_chat_id: values.deliver_chat_id || undefined,
        deliver_only: values.deliver_only === "true",
        script: values.script || undefined,
        secret: values.secret || undefined,
      });
      this.resetForm();
      await this.refresh();
      this.status(reply.message);
    } catch (error) {
      this.status(
        t.webhooks.createFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
    }
  }

  private async remove(name: string): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    try {
      await client.webhooksDelete(name);
      await this.refresh();
      this.status(t.webhooks.removed.replace("{name}", name));
    } catch (error) {
      this.status(
        t.webhooks.removeFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
    }
  }

  private async test(name: string): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    this.status(t.webhooks.testing);
    try {
      const reply = await client.webhooksTest(name);
      this.status(reply.message);
    } catch (error) {
      this.status(
        t.webhooks.testFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
    } finally {
      this.busy = false;
    }
  }
}
