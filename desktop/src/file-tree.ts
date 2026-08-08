// Chat file-tree sidebar — hermes desktop right-sidebar file tree port:
// a lazy-loading directory tree over the gateway /api/fs endpoints,
// rooted at the open session's cwd (else the gateway default cwd),
// with click-to-preview text files (read + atomic save) and download
// links for everything else. Toggle from the chat header or palette.

import type { GatewayClient } from "./gateway";
import { t } from "./i18n";

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

export class FileTreePanel {
  private aside: HTMLElement;
  private body: HTMLElement;
  private rootLabel: HTMLElement;
  private rootPath: string | null = null;

  constructor(private client: () => GatewayClient | null) {
    this.aside = document.createElement("aside");
    this.aside.id = "file-tree";
    this.aside.hidden = true;

    const header = document.createElement("div");
    header.className = "file-tree-header";
    this.rootLabel = document.createElement("span");
    this.rootLabel.className = "file-tree-root";
    this.rootLabel.textContent = t.fileTree.title;
    const refresh = document.createElement("button");
    refresh.className = "ghost";
    refresh.textContent = "\u21BB";
    refresh.title = t.kanban.refresh;
    refresh.addEventListener("click", () => {
      if (this.rootPath) void this.loadRoot(this.rootPath);
    });
    const mkdir = document.createElement("button");
    mkdir.className = "ghost";
    mkdir.textContent = "+";
    mkdir.title = t.fileTree.newFolder;
    mkdir.addEventListener("click", () => {
      void this.createFolder();
    });
    const gitRoot = document.createElement("button");
    gitRoot.className = "ghost";
    gitRoot.textContent = "\u2387";
    gitRoot.title = t.fileTree.gitRoot;
    gitRoot.addEventListener("click", () => {
      void this.jumpToGitRoot();
    });
    const close = document.createElement("button");
    close.className = "ghost";
    close.textContent = "\u2715";
    close.title = t.learning.close;
    close.addEventListener("click", () => {
      this.aside.hidden = true;
    });
    header.append(this.rootLabel, mkdir, gitRoot, refresh, close);

    this.body = document.createElement("div");
    this.body.className = "file-tree-body";

    this.aside.append(header, this.body);
  }

  mount(parent: HTMLElement): void {
    parent.appendChild(this.aside);
  }

  get visible(): boolean {
    return !this.aside.hidden;
  }

  hide(): void {
    this.aside.hidden = true;
  }

  /** Toggle the panel; resolves the root lazily on first open. */
  async toggle(cwd: string | null): Promise<void> {
    if (this.visible) {
      this.aside.hidden = true;
      return;
    }
    this.aside.hidden = false;
    await this.open(cwd);
  }

  /** Open (or re-root) the tree at the session cwd. */
  async open(cwd: string | null): Promise<void> {
    const client = this.client();
    if (!client) return;
    let root = cwd && cwd.trim() ? cwd.trim() : null;
    if (!root) {
      try {
        const reply = await client.fsDefaultCwd();
        root = reply.cwd || null;
      } catch {
        root = null;
      }
    }
    if (!root) {
      this.body.innerHTML = "";
      const note = document.createElement("p");
      note.className = "config-note";
      note.textContent = t.fileTree.noRoot;
      this.body.appendChild(note);
      return;
    }
    await this.loadRoot(root);
  }

  /** P591: create a folder under the current root and re-list. */
  private async createFolder(): Promise<void> {
    const client = this.client();
    if (!client || !this.rootPath) return;
    const name = window.prompt(t.fileTree.newFolderPrompt);
    if (!name || !name.trim()) return;
    const path = `${this.rootPath.replace(/[\\/]+$/, "")}/${name.trim()}`;
    try {
      await client.fsMkdir(path);
      await this.loadRoot(this.rootPath);
    } catch (error) {
      window.alert(
        t.fileTree.mkdirFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  /** P593: re-root the tree at the nearest enclosing git checkout. */
  private async jumpToGitRoot(): Promise<void> {
    const client = this.client();
    if (!client || !this.rootPath) return;
    try {
      const { root } = await client.fsGitRoot(this.rootPath);
      if (root && root !== this.rootPath) {
        await this.loadRoot(root);
      }
    } catch {
      // Keep the current root when the lookup fails.
    }
  }

  private async loadRoot(root: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    this.rootPath = root;
    this.rootLabel.textContent = basename(root);
    this.rootLabel.title = root;
    this.body.innerHTML = "";
    await this.renderDir(this.body, root, 0);
  }

  private async renderDir(container: HTMLElement, path: string, depth: number): Promise<void> {
    const client = this.client();
    if (!client) return;
    try {
      const { entries } = await client.fsList(path);
      if (entries.length === 0) {
        const empty = document.createElement("div");
        empty.className = "file-tree-empty";
        empty.style.paddingLeft = `${depth * 14 + 8}px`;
        empty.textContent = t.fileTree.empty;
        container.appendChild(empty);
        return;
      }
      for (const entry of entries) {
        if (entry.isDirectory) {
          const details = document.createElement("details");
          details.className = "file-tree-dir";
          const summary = document.createElement("summary");
          summary.style.paddingLeft = `${depth * 14 + 8}px`;
          summary.textContent = `\u{1F4C1} ${entry.name}`;
          summary.title = entry.path;
          const child = document.createElement("div");
          child.className = "file-tree-children";
          let loaded = false;
          details.addEventListener("toggle", () => {
            if (details.open && !loaded) {
              loaded = true;
              void this.renderDir(child, entry.path, depth + 1);
            }
          });
          details.append(summary, child);
          container.appendChild(details);
        } else {
          const row = document.createElement("button");
          row.className = "file-tree-file";
          row.style.paddingLeft = `${depth * 14 + 8}px`;
          row.textContent = `\u{1F4C4} ${entry.name}`;
          row.title = entry.path;
          row.addEventListener("click", () => {
            void this.previewFile(entry.path);
          });
          container.appendChild(row);
        }
      }
    } catch (error) {
      const note = document.createElement("div");
      note.className = "file-tree-empty";
      note.style.paddingLeft = `${depth * 14 + 8}px`;
      note.textContent = t.fileTree.loadFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
      container.appendChild(note);
    }
  }

  private async previewFile(path: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    const dialog = document.createElement("dialog");
    dialog.className = "file-tree-preview";
    const header = document.createElement("h2");
    header.textContent = basename(path);
    header.title = path;
    const status = document.createElement("div");
    status.className = "config-status";
    status.hidden = true;
    const area = document.createElement("textarea");
    area.className = "file-tree-preview-text";
    area.rows = 20;
    area.spellcheck = false;
    const buttons = document.createElement("div");
    buttons.className = "file-tree-preview-actions";
    const saveBtn = document.createElement("button");
    saveBtn.className = "primary";
    saveBtn.textContent = t.fileTree.save;
    const downloadLink = document.createElement("a");
    downloadLink.className = "ghost file-tree-download";
    downloadLink.textContent = t.fileTree.download;
    downloadLink.href = client.fsDownloadUrl(path);
    downloadLink.target = "_blank";
    downloadLink.rel = "noreferrer";
    const closeBtn = document.createElement("button");
    closeBtn.className = "ghost";
    closeBtn.textContent = t.learning.close;
    closeBtn.addEventListener("click", () => dialog.close());
    buttons.append(saveBtn, downloadLink, closeBtn);
    dialog.append(header, status, area, buttons);
    document.body.appendChild(dialog);
    dialog.addEventListener("close", () => dialog.remove());
    dialog.showModal();

    const report = (message: string, isError = false): void => {
      status.hidden = !message;
      status.textContent = message;
      status.classList.toggle("error", isError);
    };
    try {
      const file = await client.fsReadText(path);
      if (file.binary) {
        area.value = t.fileTree.binary;
        area.readOnly = true;
        saveBtn.disabled = true;
        return;
      }
      area.value = file.text;
      if (file.truncated) report(t.fileTree.truncated);
      saveBtn.addEventListener("click", () => {
        void client
          .fsWriteText(path, area.value)
          .then(() => report(t.fileTree.saved))
          .catch((error: unknown) =>
            report(
              t.fileTree.saveFailed.replace(
                "{error}",
                error instanceof Error ? error.message : String(error),
              ),
              true,
            ),
          );
      });
    } catch (error) {
      area.value = "";
      area.readOnly = true;
      saveBtn.disabled = true;
      report(
        t.fileTree.previewFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
        true,
      );
    }
  }
}
