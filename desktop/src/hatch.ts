// Desktop hatch overlay — GUI twin of `ulnclaw pets hatch` / the REPL
// `/hatch` flow, riding the gateway's long-running hatch jobs:
// prompt → base drafts → pick → row generation → adopted pet.
// Hermes parity: apps/desktop/src/app/pet-generate (rebuilt as a
// dependency-free <dialog> for the Tauri shell).

import type { GatewayClient, HatchJobStatus } from "./gateway";

const POLL_MS = 1500;

const STYLES: [string, string][] = [
  ["auto", "Pixel art (hermes default)"],
  ["pixel", "Pixel art"],
  ["plush", "Plush toy"],
  ["clay", "Claymation"],
  ["sticker", "Glossy sticker"],
  ["flat-vector", "Flat vector"],
  ["3d-toy", "3D toy"],
  ["painterly", "Painterly"],
];

export class HatchOverlay {
  private dialog: HTMLDialogElement;
  private body: HTMLDivElement;
  private pollTimer: number | null = null;
  private jobId: string | null = null;
  private blobUrls: string[] = [];

  constructor(
    private client: () => GatewayClient | null,
    private onHatched: () => void = () => {},
  ) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "hatch-dialog";
    const header = document.createElement("div");
    header.className = "hatch-header";
    const title = document.createElement("h2");
    title.textContent = "🥚 Hatch a pet";
    header.appendChild(title);
    this.body = document.createElement("div");
    this.body.className = "hatch-body";
    this.dialog.append(header, this.body);
    this.dialog.addEventListener("close", () => this.stopPolling());
    document.body.appendChild(this.dialog);
  }

  open(): void {
    this.renderForm("");
    this.dialog.showModal();
  }

  private releaseBlobs(): void {
    for (const url of this.blobUrls) URL.revokeObjectURL(url);
    this.blobUrls = [];
  }

  private stopPolling(): void {
    if (this.pollTimer !== null) {
      window.clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  private clear(): void {
    this.stopPolling();
    this.releaseBlobs();
    this.body.replaceChildren();
  }

  /** The prompt form (idle step). Keeps the last prompt on retry. */
  private renderForm(prompt: string): void {
    this.clear();
    const client = this.client();

    const hint = document.createElement("p");
    hint.className = "hatch-hint";
    hint.textContent =
      "Describe a pet; the image model sketches base looks, you pick one, " +
      "and the hatch pipeline draws every animation row (a few minutes).";
    this.body.appendChild(hint);

    const promptInput = document.createElement("textarea");
    promptInput.rows = 2;
    promptInput.placeholder = "a tiny cyber fox with neon accents";
    promptInput.value = prompt;
    this.body.appendChild(promptInput);

    const options = document.createElement("div");
    options.className = "hatch-options";

    const styleSelect = document.createElement("select");
    for (const [value, label] of STYLES) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      styleSelect.appendChild(option);
    }
    const styleLabel = document.createElement("label");
    styleLabel.append("Style ", styleSelect);

    const draftSelect = document.createElement("select");
    for (const count of [1, 2, 3, 4]) {
      const option = document.createElement("option");
      option.value = String(count);
      option.textContent = `${count} draft${count > 1 ? "s" : ""}`;
      if (count === 2) option.selected = true;
      draftSelect.appendChild(option);
    }
    const draftLabel = document.createElement("label");
    draftLabel.append("Drafts ", draftSelect);

    const nameInput = document.createElement("input");
    nameInput.type = "text";
    nameInput.placeholder = "Name (optional)";
    options.append(styleLabel, draftLabel, nameInput);
    this.body.appendChild(options);

    const actions = document.createElement("div");
    actions.className = "hatch-actions";
    const cancelBtn = document.createElement("button");
    cancelBtn.className = "ghost";
    cancelBtn.textContent = "Cancel";
    cancelBtn.onclick = () => this.dialog.close();
    const hatchBtn = document.createElement("button");
    hatchBtn.className = "primary";
    hatchBtn.textContent = "Hatch";
    hatchBtn.disabled = !client;
    if (!client) hatchBtn.title = "gateway offline";
    actions.append(cancelBtn, hatchBtn);
    this.body.appendChild(actions);

    hatchBtn.onclick = () => {
      const text = promptInput.value.trim();
      if (!text || !client) return;
      hatchBtn.disabled = true;
      client
        .hatchStart({
          prompt: text,
          style: styleSelect.value,
          name: nameInput.value.trim() || undefined,
          drafts: Number(draftSelect.value),
        })
        .then(({ job_id }) => {
          this.jobId = job_id;
          this.renderWaiting("Designing base looks…");
          this.startPolling(text);
        })
        .catch((err: unknown) => {
          hatchBtn.disabled = false;
          this.renderError(err instanceof Error ? err.message : String(err), text);
        });
    };
  }

  /** Spinner step while drafts/hatch rows are generating. */
  private renderWaiting(label: string): void {
    this.clear();
    const spinner = document.createElement("div");
    spinner.className = "hatch-spinner";
    const text = document.createElement("p");
    text.textContent = label;
    this.body.append(spinner, text);
    this.body.appendChild(this.cancelButton());
  }

  /** Draft grid step: click a base look to hatch it. */
  private renderDrafts(job: HatchJobStatus): void {
    this.clear();
    const client = this.client();
    if (!client) return;

    const hint = document.createElement("p");
    hint.className = "hatch-hint";
    hint.textContent = "Pick the base look you like best — it anchors every animation row.";
    this.body.appendChild(hint);

    const grid = document.createElement("div");
    grid.className = "hatch-drafts";
    this.body.appendChild(grid);

    const urls = job.drafts || [];
    urls.forEach(async (pathName, index) => {
      const card = document.createElement("button");
      card.className = "hatch-draft";
      card.textContent = "…";
      grid.appendChild(card);
      try {
        const blobUrl = await client.hatchImageBlob(pathName);
        this.blobUrls.push(blobUrl);
        card.replaceChildren();
        const image = document.createElement("img");
        image.src = blobUrl;
        image.alt = `draft ${index + 1}`;
        card.appendChild(image);
        card.onclick = () => {
          for (const sibling of Array.from(grid.children)) {
            (sibling as HTMLButtonElement).disabled = true;
          }
          this.pickDraft(index);
        };
      } catch {
        card.textContent = "failed to load";
      }
    });

    const actions = document.createElement("div");
    actions.className = "hatch-actions";
    const backBtn = document.createElement("button");
    backBtn.className = "ghost";
    backBtn.textContent = "Start over";
    backBtn.onclick = () => {
      const prompt = job.prompt || "";
      if (this.jobId) void client.hatchCancel(this.jobId);
      this.jobId = null;
      this.renderForm(prompt);
    };
    actions.appendChild(backBtn);
    this.body.appendChild(actions);
  }

  private pickDraft(index: number): void {
    const client = this.client();
    if (!client || !this.jobId) return;
    client
      .hatchPick(this.jobId, index)
      .then(() => {
        this.renderWaiting("Drawing animation rows…");
        this.startPolling();
      })
      .catch((err: unknown) => {
        this.renderError(err instanceof Error ? err.message : String(err));
      });
  }

  /** Hatch step: live progress lines (CLI parity) + cancel. */
  private renderProgress(job: HatchJobStatus): void {
    this.clear();
    const list = document.createElement("div");
    list.className = "hatch-progress";
    const seen = new Set<string>();
    const lines: string[] = [];
    for (const entry of job.progress || []) {
      const line = this.describeProgress(entry.event, entry.detail);
      if (line && !seen.has(line)) {
        seen.add(line);
        lines.push(line);
      }
    }
    for (const line of lines.slice(-8)) {
      const row = document.createElement("div");
      row.className = "hatch-progress-row";
      row.textContent = `┊ ${line}`;
      list.appendChild(row);
    }
    const spinner = document.createElement("div");
    spinner.className = "hatch-spinner small";
    this.body.append(list, spinner, this.cancelButton());
  }

  private describeProgress(event: string, detail: string): string | null {
    if (event === "row") {
      if (detail === "idle-fallback") return "idle (fallback frame)";
      const state = detail.split(":")[0];
      return `drawing ${state}…`;
    }
    if (event === "compose") return "composing spritesheet…";
    if (event === "save") return "saving…";
    return null;
  }

  /** Done step: spritesheet preview + adopt confirmation. */
  private renderDone(job: HatchJobStatus): void {
    this.clear();
    const client = this.client();
    const result = job.result;
    if (!client || !result) {
      this.renderError("hatch finished without a result");
      return;
    }

    const name = document.createElement("h3");
    name.textContent = `(^_^)b ${result.display_name} hatched and adopted!`;
    this.body.appendChild(name);

    const preview = document.createElement("div");
    preview.className = "hatch-preview";
    preview.textContent = "loading spritesheet…";
    this.body.appendChild(preview);
    client
      .hatchImageBlob(result.spritesheet)
      .then((blobUrl) => {
        this.blobUrls.push(blobUrl);
        const image = document.createElement("img");
        image.src = blobUrl;
        image.alt = `${result.display_name} spritesheet`;
        preview.replaceChildren(image);
      })
      .catch(() => {
        preview.textContent = "spritesheet preview unavailable";
      });

    const meta = document.createElement("p");
    meta.className = "hatch-hint";
    meta.textContent = `${result.states.length} animation rows — it'll pop into the corner shortly.`;
    this.body.appendChild(meta);

    const actions = document.createElement("div");
    actions.className = "hatch-actions";
    const doneBtn = document.createElement("button");
    doneBtn.className = "primary";
    doneBtn.textContent = "Done";
    doneBtn.onclick = () => {
      this.dialog.close();
      this.onHatched();
    };
    actions.appendChild(doneBtn);
    this.body.appendChild(actions);
  }

  private renderError(message: string, prompt = ""): void {
    this.clear();
    const text = document.createElement("p");
    text.className = "hatch-error";
    text.textContent = `(x_x) ${message}`;
    this.body.appendChild(text);
    const actions = document.createElement("div");
    actions.className = "hatch-actions";
    const retryBtn = document.createElement("button");
    retryBtn.className = "primary";
    retryBtn.textContent = "Try again";
    retryBtn.onclick = () => this.renderForm(prompt);
    const closeBtn = document.createElement("button");
    closeBtn.className = "ghost";
    closeBtn.textContent = "Close";
    closeBtn.onclick = () => this.dialog.close();
    actions.append(retryBtn, closeBtn);
    this.body.appendChild(actions);
  }

  private cancelButton(): HTMLButtonElement {
    const button = document.createElement("button");
    button.className = "ghost hatch-cancel";
    button.textContent = "Cancel hatch";
    button.onclick = () => {
      const client = this.client();
      if (client && this.jobId) void client.hatchCancel(this.jobId);
      this.dialog.close();
    };
    return button;
  }

  private startPolling(prompt = ""): void {
    this.stopPolling();
    const tick = async () => {
      const client = this.client();
      if (!client || !this.jobId) {
        this.stopPolling();
        return;
      }
      let job: HatchJobStatus;
      try {
        job = await client.hatchJob(this.jobId);
      } catch {
        return; // transient — keep polling
      }
      switch (job.status) {
        case "generating_drafts":
          this.renderWaiting("Designing base looks…");
          break;
        case "awaiting_pick":
          this.stopPolling();
          this.renderDrafts(job);
          break;
        case "hatching":
          this.renderProgress(job);
          break;
        case "done":
          this.stopPolling();
          this.renderDone(job);
          break;
        case "cancelled":
          this.stopPolling();
          this.renderError("hatch cancelled", prompt);
          break;
        case "failed":
          this.stopPolling();
          this.renderError(job.error || "hatch failed", prompt);
          break;
      }
    };
    void tick();
    this.pollTimer = window.setInterval(() => void tick(), POLL_MS);
  }
}
