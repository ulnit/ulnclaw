// Desktop artifacts browser — GUI twin of hermes' artifacts view
// (apps/desktop/src/app/artifacts/): scans recent session transcripts
// client-side for links, files, and images produced by the agent and
// renders a filterable list. Scoped port: overlay dialog instead of a
// routed page, bounded session/message scan.

import type { GatewayClient, SessionRow } from "./gateway";
import { t } from "./i18n";

export type ArtifactKind = "image" | "file" | "link";
export type ArtifactFilter = "all" | ArtifactKind;
const FILTERS: readonly ArtifactFilter[] = ["all", "image", "file", "link"];

interface ArtifactRecord {
  kind: ArtifactKind;
  value: string;
  label: string;
  sessionId: string;
  sessionTitle: string;
  timestamp: number;
}

const MARKDOWN_IMAGE_RE = /!\[([^\]]*)\]\(([^)\s]+)\)/g;
const MARKDOWN_LINK_RE = /\[([^\]]+)\]\(([^)\s]+)\)/g;
const URL_RE = /https?:\/\/[^\s<>"')]+/g;
const PATH_RE = /(^|[\s("'`])((?:\/|~\/|\.\.?\/)[^\s"'`<>]+(?:\.[a-z0-9]{1,8})?)/gi;
const IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|webp|svg|bmp)(?:\?.*)?$/i;
const FILE_EXT_RE =
  /\.(?:png|jpe?g|gif|webp|svg|bmp|pdf|txt|json|md|csv|zip|tar|gz|mp3|wav|mp4|mov)(?:\?.*)?$/i;

/** Cap the transcript scan so large gateways stay responsive. */
const MAX_SESSIONS = 30;
const MAX_MESSAGES = 200;

function normalizeValue(value: string): string {
  return value.trim().replace(/[),.;]+$/, "");
}

function looksLikeArtifact(value: string): boolean {
  if (/^(?:https?:\/\/|data:image\/)/.test(value)) return true;
  const pathy =
    value.startsWith("/") ||
    value.startsWith("./") ||
    value.startsWith("../") ||
    value.startsWith("~/") ||
    value.startsWith("file://");
  if (pathy && (IMAGE_EXT_RE.test(value) || FILE_EXT_RE.test(value))) return true;
  return value.startsWith("/") && value.includes(".");
}

function artifactKind(value: string): ArtifactKind {
  if (value.startsWith("data:image/") || IMAGE_EXT_RE.test(value)) return "image";
  if (
    value.startsWith("/") ||
    value.startsWith("./") ||
    value.startsWith("../") ||
    value.startsWith("~/") ||
    value.startsWith("file://")
  ) {
    return "file";
  }
  return "link";
}

function artifactLabel(value: string): string {
  try {
    const url = new URL(value);
    const item = url.pathname.split("/").filter(Boolean).pop();
    return item || value;
  } catch {
    const parts = value.split(/[\\/]/).filter(Boolean);
    return parts.pop() || value;
  }
}

/** Extract artifact candidates from one message body. */
export function collectFromText(text: string): string[] {
  const found = new Set<string>();
  for (const match of text.matchAll(MARKDOWN_IMAGE_RE)) found.add(normalizeValue(match[2]));
  for (const match of text.matchAll(MARKDOWN_LINK_RE)) found.add(normalizeValue(match[2]));
  for (const match of text.matchAll(URL_RE)) found.add(normalizeValue(match[0]));
  for (const match of text.matchAll(PATH_RE)) found.add(normalizeValue(match[2]));
  return [...found].filter(looksLikeArtifact);
}

export class ArtifactsOverlay {
  private dialog: HTMLDialogElement;
  private body: HTMLDivElement;
  private records: ArtifactRecord[] = [];
  private filter: ArtifactFilter = "all";
  private query = "";

  constructor(
    private client: () => GatewayClient | null,
    private sessions: () => SessionRow[],
    private openSession: (id: string) => void,
  ) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "artifacts-dialog";
    const header = document.createElement("div");
    header.className = "artifacts-header";
    const title = document.createElement("h2");
    title.textContent = t.artifacts.title;
    header.appendChild(title);
    this.body = document.createElement("div");
    this.body.className = "artifacts-body";
    this.dialog.append(header, this.body);
    document.body.appendChild(this.dialog);
  }

  async open(): Promise<void> {
    this.filter = "all";
    this.query = "";
    this.records = [];
    this.body.innerHTML = "";
    const loading = document.createElement("div");
    loading.className = "artifacts-loading";
    loading.textContent = t.artifacts.scanning;
    this.body.appendChild(loading);
    this.dialog.showModal();

    const client = this.client();
    if (!client) {
      loading.textContent = t.artifacts.notConnected;
      return;
    }
    try {
      const sessions = [...this.sessions()]
        .sort((a, b) => b.last_activity_at - a.last_activity_at)
        .slice(0, MAX_SESSIONS);
      const records: ArtifactRecord[] = [];
      const seen = new Set<string>();
      for (const session of sessions) {
        let messages;
        try {
          messages = await client.messages(session.id);
        } catch {
          continue;
        }
        const title = session.title || session.id.slice(0, 8);
        for (const message of messages.slice(-MAX_MESSAGES)) {
          const content = message.content;
          if (!content) continue;
          for (const value of collectFromText(content)) {
            const key = `${session.id}\u0000${value}`;
            if (seen.has(key)) continue;
            seen.add(key);
            records.push({
              kind: artifactKind(value),
              value,
              label: artifactLabel(value),
              sessionId: session.id,
              sessionTitle: title,
              timestamp: session.last_activity_at,
            });
          }
        }
      }
      this.records = records.sort((a, b) => b.timestamp - a.timestamp);
      this.render();
    } catch (err) {
      loading.textContent = t.artifacts.scanFailed.replace("{error}", String(err));
    }
  }

  private visible(): ArtifactRecord[] {
    const query = this.query.trim().toLowerCase();
    return this.records.filter((record) => {
      if (this.filter !== "all" && record.kind !== this.filter) return false;
      if (!query) return true;
      return (
        record.value.toLowerCase().includes(query) ||
        record.sessionTitle.toLowerCase().includes(query)
      );
    });
  }

  private render(): void {
    this.body.innerHTML = "";

    const toolbar = document.createElement("div");
    toolbar.className = "artifacts-toolbar";
    for (const filter of FILTERS) {
      const btn = document.createElement("button");
      btn.className = "artifacts-filter" + (filter === this.filter ? " active" : "");
      btn.textContent = filter;
      btn.onclick = () => {
        this.filter = filter;
        this.render();
      };
      toolbar.appendChild(btn);
    }
    const search = document.createElement("input");
    search.className = "artifacts-search";
    search.placeholder = t.artifacts.filterPlaceholder;
    search.value = this.query;
    search.addEventListener("input", () => {
      this.query = search.value;
      this.renderList();
    });
    toolbar.appendChild(search);
    this.body.appendChild(toolbar);

    const list = document.createElement("div");
    list.className = "artifacts-list";
    this.body.appendChild(list);
    this.renderList();
  }

  private renderList(): void {
    const list = this.body.querySelector<HTMLElement>(".artifacts-list");
    if (!list) return;
    list.innerHTML = "";
    const visible = this.visible().slice(0, 200);
    if (visible.length === 0) {
      const empty = document.createElement("div");
      empty.className = "artifacts-empty";
      empty.textContent = t.artifacts.none;
      list.appendChild(empty);
      return;
    }
    for (const record of visible) {
      const row = document.createElement("div");
      row.className = "artifacts-row";
      const icon = document.createElement("span");
      icon.className = "artifacts-icon";
      icon.textContent = record.kind === "image" ? "🖼️" : record.kind === "file" ? "📄" : "🔗";
      const main = document.createElement("div");
      main.className = "artifacts-main";
      const label = document.createElement("div");
      label.className = "artifacts-label";
      label.textContent = record.label;
      const value = document.createElement("div");
      value.className = "artifacts-value";
      value.textContent = record.value;
      main.append(label, value);
      const meta = document.createElement("div");
      meta.className = "artifacts-meta";
      const session = document.createElement("button");
      session.className = "artifacts-session";
      session.textContent = record.sessionTitle;
      session.title = t.artifacts.openSession;
      session.onclick = (event) => {
        event.stopPropagation();
        this.dialog.close();
        this.openSession(record.sessionId);
      };
      const when = document.createElement("span");
      when.textContent = new Date(record.timestamp * 1000).toLocaleString();
      meta.append(session, when);
      row.append(icon, main, meta);
      row.onclick = () => void this.activate(record);
      list.appendChild(row);
    }
  }

  private async activate(record: ArtifactRecord): Promise<void> {
    if (record.kind === "link" || record.value.startsWith("http")) {
      window.open(record.value, "_blank", "noopener");
      return;
    }
    // Images inside the gateway's media roots are served over
    // GET /api/media (P338) — open them directly instead of copying.
    if (record.kind === "image" && !record.value.startsWith("data:")) {
      const client = this.client();
      if (client) {
        try {
          const dataUrl = await client.mediaDataUrl(record.value);
          const response = await fetch(dataUrl);
          const blob = await response.blob();
          const objectUrl = URL.createObjectURL(blob);
          window.open(objectUrl, "_blank");
          return;
        } catch {
          // Outside the media roots / not servable — fall back to copy.
        }
      }
    }
    // Files/images live on the gateway host — copy the path for the
    // operator (no arbitrary fs access from the webview).
    try {
      await navigator.clipboard.writeText(record.value);
      this.toast(`Copied: ${record.value}`);
    } catch {
      this.toast(record.value);
    }
  }

  private toast(text: string): void {
    const note = document.createElement("div");
    note.className = "artifacts-toast";
    note.textContent = text;
    this.dialog.appendChild(note);
    setTimeout(() => note.remove(), 2500);
  }
}
