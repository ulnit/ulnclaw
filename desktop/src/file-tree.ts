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
  private tabs: HTMLElement;
  private filesTabBtn: HTMLButtonElement;
  private changesTabBtn: HTMLButtonElement;
  private rootPath: string | null = null;
  private tab: "files" | "changes" = "files";
  private diffMode: "working" | "staged" | "all" = "working";
  private reviewNote: { text: string; error: boolean } | null = null;
  private lastStatus: { staged: string[]; unstaged: string[]; untracked: string[] } | null = null;

  constructor(
    private client: () => GatewayClient | null,
    /** P594: attach a file to the chat composer (path + display name). */
    private attach: ((path: string, name: string) => void) | null = null,
  ) {
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

    this.tabs = document.createElement("div");
    this.tabs.className = "file-tree-tabs";
    this.filesTabBtn = document.createElement("button");
    this.filesTabBtn.textContent = t.fileTree.filesTab;
    this.filesTabBtn.addEventListener("click", () => this.switchTab("files"));
    this.changesTabBtn = document.createElement("button");
    this.changesTabBtn.textContent = t.fileTree.changesTab;
    this.changesTabBtn.addEventListener("click", () => this.switchTab("changes"));
    this.tabs.append(this.filesTabBtn, this.changesTabBtn);

    this.aside.append(header, this.tabs, this.body);
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
    if (this.tab === "changes") {
      await this.renderChanges();
    } else {
      await this.renderDir(this.body, root, 0);
    }
  }

  /** P596: switch between the files tree and the git-changes pane. */
  private switchTab(tab: "files" | "changes"): void {
    this.reviewNote = null;
    if (this.tab === tab) return;
    this.tab = tab;
    this.filesTabBtn.classList.toggle("active", tab === "files");
    this.changesTabBtn.classList.toggle("active", tab === "changes");
    this.body.innerHTML = "";
    if (tab === "files") {
      if (this.rootPath) void this.renderDir(this.body, this.rootPath, 0);
    } else {
      void this.renderChanges();
    }
  }

  /** P596: render the working-tree diff (stat + untracked + patch). */
  private async renderChanges(): Promise<void> {
    const client = this.client();
    if (!client || !this.rootPath) return;
    this.body.innerHTML = "";
    this.filesTabBtn.classList.toggle("active", false);
    this.changesTabBtn.classList.toggle("active", true);
    const bar = document.createElement("div");
    bar.className = "file-tree-changes-bar";
    const mode = document.createElement("select");
    const options: ["working" | "staged" | "all", string][] = [
      ["working", t.fileTree.diffWorking],
      ["staged", t.fileTree.diffStaged],
      ["all", t.fileTree.diffAll],
    ];
    for (const [value, label] of options) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      if (value === this.diffMode) option.selected = true;
      mode.appendChild(option);
    }
    mode.addEventListener("change", () => {
      this.diffMode = mode.value as "working" | "staged" | "all";
      void this.renderChanges();
    });
    bar.appendChild(mode);
    this.body.appendChild(bar);
    await this.renderReviewBar();
    const status = document.createElement("div");
    status.className = "config-note";
    status.textContent = t.learning.loading;
    this.body.appendChild(status);
    try {
      const diff = await client.fsGitDiff(this.rootPath, this.diffMode);
      status.remove();
      this.renderFileGroups();
      if (diff.empty && diff.untracked.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = t.fileTree.noChanges;
        this.body.appendChild(empty);
        return;
      }
      if (diff.stat) {
        const stat = document.createElement("pre");
        stat.className = "file-tree-diff";
        stat.textContent = diff.stat;
        this.body.appendChild(stat);
      }
      if (diff.untracked.length > 0) {
        const untracked = document.createElement("pre");
        untracked.className = "file-tree-diff";
        untracked.textContent =
          t.fileTree.untracked +
          "\n" +
          diff.untracked.map((path) => `+ ${path}`).join("\n");
        this.body.appendChild(untracked);
      }
      if (diff.diff) {
        const pre = document.createElement("pre");
        pre.className = "file-tree-diff";
        const lines = diff.diff.split("\n");
        pre.textContent = lines.length > 4000
          ? lines.slice(0, 4000).join("\n") + `\n\u2026 (${lines.length - 4000} more lines)`
          : diff.diff;
        this.body.appendChild(pre);
      }
    } catch (error) {
      status.textContent = t.fileTree.diffFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /**
   * Per-file change groups (P600) — staged / modified / untracked rows
   * with per-row stage/unstage/revert actions over the same
   * `/api/git/*` endpoints (hermes per-file review parity).
   */
  private renderFileGroups(): void {
    const client = this.client();
    const status = this.lastStatus;
    if (!client || !this.rootPath || !status) return;
    const groups: { title: string; files: string[]; kind: "staged" | "unstaged" | "untracked" }[] = [
      { title: t.gitReview.groupStaged, files: status.staged, kind: "staged" },
      { title: t.gitReview.groupModified, files: status.unstaged, kind: "unstaged" },
      { title: t.gitReview.groupUntracked, files: status.untracked, kind: "untracked" },
    ];
    const act = async (
      fn: (path: string) => Promise<unknown>,
      file: string,
      noteText: string,
    ): Promise<void> => {
      try {
        await fn(file);
        this.reviewNote = { text: noteText, error: false };
        await this.renderChanges();
      } catch (error) {
        this.reviewNote = {
          text: t.gitReview.failed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          ),
          error: true,
        };
        await this.renderChanges();
      }
    };
    for (const group of groups) {
      if (group.files.length === 0) continue;
      const details = document.createElement("details");
      details.className = "file-tree-group";
      details.open = true;
      const summary = document.createElement("summary");
      summary.textContent = `${group.title.replace("{count}", String(group.files.length))}`;
      details.appendChild(summary);
      for (const file of group.files) {
        const row = document.createElement("div");
        row.className = "file-tree-group-row";
        const label = document.createElement("span");
        label.className = "file-tree-group-file";
        label.textContent = file;
        label.title = file;
        row.appendChild(label);
        if (group.kind === "staged") {
          const unstage = document.createElement("button");
          unstage.className = "ghost";
          unstage.textContent = "\u2212";
          unstage.title = t.gitReview.unstageAll;
          unstage.addEventListener("click", () =>
            void act(
              (path) => client.gitUnstage(this.rootPath!, [path]),
              file,
              t.gitReview.unstagedNote,
            ),
          );
          row.appendChild(unstage);
        } else if (group.kind === "unstaged") {
          const stage = document.createElement("button");
          stage.className = "ghost";
          stage.textContent = "\uff0b";
          stage.title = t.gitReview.stageAll;
          stage.addEventListener("click", () =>
            void act(
              (path) => client.gitStage(this.rootPath!, [path]),
              file,
              t.gitReview.stagedNote,
            ),
          );
          const revert = document.createElement("button");
          revert.className = "ghost";
          revert.textContent = "\u21ba";
          revert.title = t.gitReview.revert;
          revert.addEventListener("click", () => {
            if (!window.confirm(t.gitReview.revertConfirm.replace("{count}", "1"))) return;
            void act(
              (path) => client.gitRevert(this.rootPath!, [path]),
              file,
              t.gitReview.reverted.replace("{count}", "1"),
            );
          });
          row.append(stage, revert);
        } else {
          const stage = document.createElement("button");
          stage.className = "ghost";
          stage.textContent = "\uff0b";
          stage.title = t.gitReview.stageAll;
          stage.addEventListener("click", () =>
            void act(
              (path) => client.gitStage(this.rootPath!, [path]),
              file,
              t.gitReview.stagedNote,
            ),
          );
          row.appendChild(stage);
        }
        details.appendChild(row);
      }
      this.body.appendChild(details);
    }
  }

  /**
   * Git review action bar (P598) — branch/ahead-behind/counts plus
   * stage/unstage/revert/commit/push over the `/api/git/*` endpoints
   * (hermes right-sidebar review actions parity). Hidden when the root
   * is not a git checkout.
   */
  private async renderReviewBar(): Promise<void> {
    const client = this.client();
    if (!client || !this.rootPath) return;
    let summary: {
      branch: string; upstream: string | null; ahead: number; behind: number;
      staged: string[]; unstaged: string[]; untracked: string[];
    };
    try {
      summary = await client.gitStatus(this.rootPath);
    } catch {
      return;
    }
    this.lastStatus = {
      staged: summary.staged,
      unstaged: summary.unstaged,
      untracked: summary.untracked,
    };
    const review = document.createElement("div");
    review.className = "file-tree-review";

    const line = document.createElement("div");
    line.className = "file-tree-review-status";
    const branchSelect = document.createElement("select");
    branchSelect.className = "file-tree-review-branch";
    try {
      const { branches } = await client.gitBranches(this.rootPath);
      for (const name of branches) {
        const option = document.createElement("option");
        option.value = name;
        option.textContent = name;
        if (name === summary.branch) option.selected = true;
        branchSelect.appendChild(option);
      }
    } catch {
      const option = document.createElement("option");
      option.textContent = summary.branch;
      branchSelect.appendChild(option);
    }
    const newBranchBtn = document.createElement("button");
    newBranchBtn.className = "ghost file-tree-review-newbranch";
    newBranchBtn.textContent = "\uff0b";
    newBranchBtn.title = t.gitReview.newBranch;
    newBranchBtn.addEventListener("click", () => {
      const raw = window.prompt(t.gitReview.newBranchPrompt);
      const name = (raw || "").trim();
      if (!name) return;
      void run(async () => {
        await client.gitBranchCreate(this.rootPath!, name);
        await client.gitBranchSwitch(this.rootPath!, name);
        this.reviewNote = {
          text: t.gitReview.createdBranch.replace("{name}", name),
          error: false,
        };
      });
    });
    const aheadBehind = document.createElement("span");
    aheadBehind.className = "file-tree-review-counts";
    aheadBehind.textContent = summary.upstream
      ? ` \u2191${summary.ahead} \u2193${summary.behind}`
      : "";
    const counts = document.createElement("span");
    counts.className = "file-tree-review-counts";
    counts.textContent =
      " \u00b7 " +
      t.gitReview.counts
        .replace("{staged}", String(summary.staged.length))
        .replace("{unstaged}", String(summary.unstaged.length))
        .replace("{untracked}", String(summary.untracked.length));
    branchSelect.addEventListener("change", () => {
      const target = branchSelect.value;
      if (!target || target === summary.branch) return;
      void run(async () => {
        await client.gitBranchSwitch(this.rootPath!, target);
        this.reviewNote = {
          text: t.gitReview.switchedBranch.replace("{name}", target),
          error: false,
        };
      });
    });
    line.append(branchSelect, newBranchBtn, aheadBehind, counts);
    review.appendChild(line);

    const feedback = document.createElement("div");
    feedback.className = "file-tree-review-feedback";
    feedback.hidden = true;
    const note = (message: string, isError = false): void => {
      feedback.hidden = false;
      feedback.textContent = message;
      feedback.classList.toggle("error", isError);
    };
    const run = async (action: () => Promise<void>): Promise<void> => {
      try {
        await action();
        await this.renderChanges();
      } catch (error) {
        this.reviewNote = {
          text: t.gitReview.failed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          ),
          error: true,
        };
        note(this.reviewNote.text, true);
      }
    };
    const mkBtn = (label: string): HTMLButtonElement => {
      const button = document.createElement("button");
      button.className = "ghost";
      button.textContent = label;
      return button;
    };

    const actions = document.createElement("div");
    actions.className = "file-tree-review-actions";
    const stageBtn = mkBtn(t.gitReview.stageAll);
    stageBtn.disabled = summary.unstaged.length === 0 && summary.untracked.length === 0;
    stageBtn.addEventListener("click", () =>
      void run(async () => {
        await client.gitStage(this.rootPath!, []);
        this.reviewNote = { text: t.gitReview.stagedNote, error: false };
      }),
    );
    const unstageBtn = mkBtn(t.gitReview.unstageAll);
    unstageBtn.disabled = summary.staged.length === 0;
    unstageBtn.addEventListener("click", () =>
      void run(async () => {
        await client.gitUnstage(this.rootPath!, []);
        this.reviewNote = { text: t.gitReview.unstagedNote, error: false };
      }),
    );
    const revertBtn = mkBtn(t.gitReview.revert);
    revertBtn.disabled = summary.unstaged.length === 0;
    revertBtn.addEventListener("click", () => {
      const targets = summary.unstaged.slice();
      if (!window.confirm(t.gitReview.revertConfirm.replace("{count}", String(targets.length)))) {
        return;
      }
      void run(async () => {
        await client.gitRevert(this.rootPath!, targets);
        this.reviewNote = {
          text: t.gitReview.reverted.replace("{count}", String(targets.length)),
          error: false,
        };
      });
    });
    actions.append(stageBtn, unstageBtn, revertBtn);
    review.appendChild(actions);

    const commitRow = document.createElement("div");
    commitRow.className = "file-tree-review-commit";
    const messageInput = document.createElement("input");
    messageInput.type = "text";
    messageInput.placeholder = t.gitReview.commitPlaceholder;
    const commitBtn = mkBtn(t.gitReview.commit);
    messageInput.addEventListener("keydown", (event) => {
      event.stopPropagation();
      if (event.key === "Enter") {
        event.preventDefault();
        commitBtn.click();
      }
    });
    commitBtn.disabled = summary.staged.length === 0;
    commitBtn.addEventListener("click", () => {
      const message = messageInput.value.trim();
      if (!message) {
        messageInput.focus();
        return;
      }
      void run(async () => {
        await client.gitCommit(this.rootPath!, message);
        this.reviewNote = { text: t.gitReview.committed, error: false };
      });
    });
    commitRow.append(messageInput, commitBtn);
    review.appendChild(commitRow);

    const pushBtn = mkBtn(t.gitReview.push);
    pushBtn.classList.add("file-tree-review-push");
    pushBtn.disabled = !summary.upstream;
    pushBtn.title = summary.upstream || "";
    pushBtn.addEventListener("click", () => {
      pushBtn.disabled = true;
      pushBtn.textContent = t.gitReview.pushing;
      void run(async () => {
        await client.gitPush(this.rootPath!);
        this.reviewNote = { text: t.gitReview.pushed, error: false };
      });
    });
    review.appendChild(pushBtn);

    if (this.reviewNote) {
      note(this.reviewNote.text, this.reviewNote.error);
    }
    review.appendChild(feedback);
    this.body.appendChild(review);
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
          const row = document.createElement("div");
          row.className = "file-tree-file-row";
          row.style.paddingLeft = `${depth * 14 + 8}px`;
          const main = document.createElement("button");
          main.className = "file-tree-file";
          main.textContent = `\u{1F4C4} ${entry.name}`;
          main.title = entry.path;
          main.addEventListener("click", () => {
            void this.previewFile(entry.path);
          });
          row.appendChild(main);
          if (this.attach) {
            const attachBtn = document.createElement("button");
            attachBtn.className = "ghost file-tree-attach";
            attachBtn.textContent = "\u{1F4CE}";
            attachBtn.title = t.chrome.attachTitle;
            attachBtn.addEventListener("click", (event) => {
              event.stopPropagation();
              this.attach!(entry.path, entry.name);
            });
            row.appendChild(attachBtn);
          }
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
