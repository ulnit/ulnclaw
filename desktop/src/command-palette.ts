// Desktop command palette — GUI twin of hermes' command palette
// (apps/desktop/src/app/command-palette/): Ctrl/Cmd+K fuzzy launcher
// for navigation, session switching, and session actions. Scoped to
// the ulnclaw desktop surfaces (chat/kanban/projects/jobs views).
// Dependency-free <dialog> like the model-picker/hatch overlays.

import type { SessionRow } from "./gateway";
import { fmt, t } from "./i18n";

export interface PaletteCommand {
  id: string;
  label: string;
  group: string;
  hint?: string;
  run: () => void | Promise<void>;
}

export interface CommandPaletteHooks {
  sessions(): SessionRow[];
  currentSessionId(): string | null;
  newSession(): void;
  openSession(id: string): void | Promise<void>;
  renameSession(): void | Promise<void>;
  deleteSession(): void | Promise<void>;
  modelPicker(): void | Promise<void>;
  resumeSession(): void | Promise<void>;
  artifacts(): void | Promise<void>;
  learning(): void | Promise<void>;
  findInChat(): void;
  switchView(view: "chat" | "kanban" | "projects" | "jobs" | "usage" | "config" | "doctor" | "webhooks" | "runs" | "skills" | "sessions" | "models" | "plugins" | "pairing"): void;
  exportSession(format: "md" | "html" | "json"): Promise<void>;
  openSettings(): void;
  refreshSessions(): void | Promise<void>;
  restartGateway(): void | Promise<void>;
  shortcuts(): void;
  notifications(): void;
  updateCheck(): void | Promise<void>;
  kanbanDispatch(): void | Promise<void>;
  kanbanQuickAdd(): void | Promise<void>;
  kanbanBoardSwitcher(): void | Promise<void>;
  toggleSidebar(): void;
  themePicker(): void | Promise<void>;
  fontPicker(): void | Promise<void>;
  copySessionId(): void | Promise<void>;
  copyLastReply(): void | Promise<void>;
  copyTranscript(): void | Promise<void>;
  forkSession(): void | Promise<void>;
  retitleSkills(): void | Promise<void>;
  toggleHideArchived(): void;
  toggleUnreadOnly(): void;
  togglePinSession(): void;
  markAllRead(): void;
  newSessionInProject(): void | Promise<void>;
  archiveSession(): void | Promise<void>;
  unarchiveSession(): void | Promise<void>;
  openInSessionsBrowser(): void;
}

/** Subsequence fuzzy score (higher = better, null = no match). */
function fuzzyScore(query: string, label: string): number | null {
  if (query.length === 0) return 1;
  const q = query.toLowerCase();
  const text = label.toLowerCase();
  if (text.includes(q)) {
    // Contiguous matches win; earlier is better.
    return 1000 - text.indexOf(q) + q.length * 4;
  }
  let qi = 0;
  let score = 0;
  let streak = 0;
  for (let ti = 0; ti < text.length && qi < q.length; ti += 1) {
    if (text[ti] === q[qi]) {
      qi += 1;
      streak += 1;
      score += 2 + streak * 2 - ti * 0.01;
    } else {
      streak = 0;
    }
  }
  return qi === q.length ? score : null;
}

export class CommandPalette {
  private dialog: HTMLDialogElement;
  private input: HTMLInputElement;
  private list: HTMLDivElement;
  private items: PaletteCommand[] = [];
  private selected = 0;
  private keyHandler: (event: KeyboardEvent) => void;

  constructor(private hooks: CommandPaletteHooks) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "command-palette-dialog";
    this.input = document.createElement("input");
    this.input.className = "command-palette-input";
    this.input.placeholder = t.palette.placeholder;
    this.list = document.createElement("div");
    this.list.className = "command-palette-list";
    this.dialog.append(this.input, this.list);
    document.body.appendChild(this.dialog);

    this.input.addEventListener("input", () => this.render());
    this.input.addEventListener("keydown", (event) => this.onKey(event));
    this.dialog.addEventListener("click", (event) => {
      // Click on the backdrop closes.
      if (event.target === this.dialog) this.close();
    });
    this.keyHandler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        this.toggle();
      }
    };
    window.addEventListener("keydown", this.keyHandler);
  }

  toggle(): void {
    if (this.dialog.open) {
      this.close();
    } else {
      void this.open();
    }
  }

  async open(): Promise<void> {
    this.input.value = "";
    this.selected = 0;
    this.items = await this.buildCommands();
    this.render();
    this.dialog.showModal();
    this.input.focus();
  }

  close(): void {
    if (this.dialog.open) this.dialog.close();
  }

  private async buildCommands(): Promise<PaletteCommand[]> {
    const hooks = this.hooks;
    const commands: PaletteCommand[] = [
      { id: "new-session", label: t.palette.newSession, group: t.palette.sessionsGroup, hint: "Ctrl/Cmd+N", run: () => hooks.newSession() },
      { id: "view-chat", label: t.palette.goToChat, group: t.palette.navigate, run: () => hooks.switchView("chat") },
      { id: "view-kanban", label: t.palette.goToKanban, group: t.palette.navigate, run: () => hooks.switchView("kanban") },
      { id: "view-projects", label: t.palette.goToProjects, group: t.palette.navigate, run: () => hooks.switchView("projects") },
      { id: "view-jobs", label: t.palette.goToJobs, group: t.palette.navigate, run: () => hooks.switchView("jobs") },
      { id: "view-usage", label: t.palette.goToUsage, group: t.palette.navigate, run: () => hooks.switchView("usage") },
      { id: "view-config", label: t.palette.goToConfig, group: t.palette.navigate, run: () => hooks.switchView("config") },
      { id: "view-doctor", label: t.palette.goToDoctor, group: t.palette.navigate, run: () => hooks.switchView("doctor") },
      { id: "view-webhooks", label: t.palette.goToWebhooks, group: t.palette.navigate, run: () => hooks.switchView("webhooks") },
      { id: "view-runs", label: t.palette.goToRuns, group: t.palette.navigate, run: () => hooks.switchView("runs") },
      { id: "view-skills", label: t.palette.goToSkills, group: t.palette.navigate, run: () => hooks.switchView("skills") },
      { id: "view-sessions", label: t.palette.goToSessions, group: t.palette.navigate, run: () => hooks.switchView("sessions") },
      { id: "view-models", label: t.palette.goToModels, group: t.palette.navigate, run: () => hooks.switchView("models") },
      { id: "view-plugins", label: t.palette.goToPlugins, group: t.palette.navigate, run: () => hooks.switchView("plugins") },
      { id: "view-pairing", label: t.palette.goToPairing, group: t.palette.navigate, run: () => hooks.switchView("pairing") },
      { id: "find", label: t.palette.findInChat, group: t.palette.sessionGroup, hint: "Ctrl/Cmd+F", run: () => hooks.findInChat() },
      { id: "artifacts", label: t.palette.browseArtifacts, group: t.palette.sessionGroup, hint: t.palette.hintArtifacts, run: () => hooks.artifacts() },
      { id: "learning", label: t.palette.learningGraph, group: t.palette.sessionGroup, hint: t.palette.hintLearning, run: () => hooks.learning() },
      { id: "model", label: t.palette.modelForSession, group: t.palette.sessionGroup, hint: "Ctrl/Cmd+Shift+M", run: () => hooks.modelPicker() },
      { id: "resume", label: t.palette.resumeSession, group: t.palette.sessionGroup, hint: "/resume", run: () => hooks.resumeSession() },
      { id: "rename", label: t.palette.renameSession, group: t.palette.sessionGroup, run: () => hooks.renameSession() },
      { id: "delete", label: t.palette.deleteSession, group: t.palette.sessionGroup, run: () => hooks.deleteSession() },
      { id: "export-md", label: t.palette.exportMd, group: t.palette.sessionGroup, run: () => hooks.exportSession("md") },
      { id: "export-html", label: t.palette.exportHtml, group: t.palette.sessionGroup, run: () => hooks.exportSession("html") },
      { id: "refresh", label: t.palette.refreshSessions, group: t.palette.gatewayGroup, run: () => hooks.refreshSessions() },
      { id: "settings", label: t.palette.openSettings, group: t.palette.gatewayGroup, hint: "Ctrl/Cmd+,", run: () => hooks.openSettings() },
      { id: "restart-gateway", label: t.palette.restartGateway, group: t.palette.gatewayGroup, run: () => hooks.restartGateway() },
      { id: "update-check", label: t.palette.updateCheck, group: t.palette.gatewayGroup, run: () => hooks.updateCheck() },
      { id: "shortcuts", label: t.palette.shortcuts, group: t.palette.gatewayGroup, hint: "F1", run: () => hooks.shortcuts() },
      { id: "notifications", label: t.palette.notifications, group: t.palette.gatewayGroup, run: () => hooks.notifications() },
      { id: "kanban-dispatch", label: t.palette.kanbanDispatch, group: t.palette.gatewayGroup, run: () => hooks.kanbanDispatch() },
      { id: "kanban-quick-add", label: t.palette.kanbanQuickAdd, group: t.palette.gatewayGroup, run: () => hooks.kanbanQuickAdd() },
      { id: "kanban-board-switch", label: t.palette.kanbanBoardSwitch, group: t.palette.gatewayGroup, run: () => hooks.kanbanBoardSwitcher() },
      { id: "session-archive", label: t.palette.archiveSession, group: t.palette.gatewayGroup, run: () => hooks.archiveSession() },
      { id: "session-unarchive", label: t.palette.unarchiveSession, group: t.palette.gatewayGroup, run: () => hooks.unarchiveSession() },
      { id: "toggle-sidebar", label: t.palette.toggleSidebar, group: t.palette.gatewayGroup, hint: "Ctrl/Cmd+B", run: () => hooks.toggleSidebar() },
      { id: "theme-picker", label: t.palette.themePicker, group: t.palette.gatewayGroup, run: () => hooks.themePicker() },
      { id: "font-picker", label: t.palette.fontPicker, group: t.palette.gatewayGroup, run: () => hooks.fontPicker() },
      { id: "copy-session-id", label: t.palette.copySessionId, group: t.palette.sessionGroup, run: () => hooks.copySessionId() },
      { id: "copy-last-reply", label: t.palette.copyLastReply, group: t.palette.gatewayGroup, run: () => hooks.copyLastReply() },
      { id: "copy-transcript", label: t.palette.copyTranscript, group: t.palette.sessionGroup, run: () => hooks.copyTranscript() },
      { id: "fork-session", label: t.palette.forkSession, group: t.palette.sessionGroup, run: () => hooks.forkSession() },
      { id: "retitle-skills", label: t.palette.retitleSkills, group: t.palette.gatewayGroup, run: () => hooks.retitleSkills() },
      { id: "toggle-hide-archived", label: t.palette.toggleHideArchived, group: t.palette.sessionGroup, run: () => hooks.toggleHideArchived() },
      { id: "toggle-unread-only", label: t.palette.unreadOnly, group: t.palette.sessionGroup, run: () => hooks.toggleUnreadOnly() },
      { id: "toggle-pin", label: t.palette.pinSession, group: t.palette.sessionGroup, run: () => hooks.togglePinSession() },
      { id: "mark-all-read", label: t.palette.markAllRead, group: t.palette.sessionGroup, run: () => hooks.markAllRead() },
      { id: "new-session-in-project", label: t.palette.newSessionInProject, group: t.palette.sessionGroup, run: () => hooks.newSessionInProject() },
      { id: "view-in-sessions", label: t.palette.viewInSessionsBrowser, group: t.palette.sessionGroup, run: () => hooks.openInSessionsBrowser() },
    ];
    const current = hooks.currentSessionId();
    for (const session of hooks.sessions()) {
      const title = session.title || session.id.slice(0, 8);
      commands.push({
        id: `switch-${session.id}`,
        label: fmt(t.palette.switchTo, { title }),
        group: t.palette.switchSession,
        hint: session.id === current ? "current" : undefined,
        run: () => hooks.openSession(session.id),
      });
    }
    return commands;
  }

  private matches(): PaletteCommand[] {
    const query = this.input.value.trim();
    const scored = this.items
      .map((item) => ({ item, score: fuzzyScore(query, item.label) }))
      .filter((entry): entry is { item: PaletteCommand; score: number } => entry.score !== null)
      .sort((a, b) => b.score - a.score);
    return scored.map((entry) => entry.item).slice(0, 30);
  }

  private render(): void {
    const visible = this.matches();
    this.selected = Math.min(this.selected, Math.max(visible.length - 1, 0));
    this.list.innerHTML = "";
    let lastGroup = "";
    visible.forEach((item, index) => {
      if (item.group !== lastGroup) {
        lastGroup = item.group;
        const header = document.createElement("div");
        header.className = "command-palette-group";
        header.textContent = item.group;
        this.list.appendChild(header);
      }
      const row = document.createElement("div");
      row.className = "command-palette-item" + (index === this.selected ? " active" : "");
      const label = document.createElement("span");
      label.textContent = item.label;
      row.appendChild(label);
      if (item.hint) {
        const hint = document.createElement("span");
        hint.className = "command-palette-hint";
        hint.textContent = item.hint;
        row.appendChild(hint);
      }
      row.addEventListener("mouseenter", () => {
        this.selected = index;
        this.highlight(visible);
      });
      row.onclick = () => void this.runItem(item);
      this.list.appendChild(row);
    });
    if (visible.length === 0) {
      const empty = document.createElement("div");
      empty.className = "command-palette-empty";
      empty.textContent = t.palette.noMatches;
      this.list.appendChild(empty);
    }
  }

  private highlight(visible: PaletteCommand[]): void {
    const rows = this.list.querySelectorAll<HTMLElement>(".command-palette-item");
    rows.forEach((row, index) => row.classList.toggle("active", index === this.selected));
  }

  private onKey(event: KeyboardEvent): void {
    const visible = this.matches();
    if (event.key === "ArrowDown") {
      event.preventDefault();
      this.selected = Math.min(this.selected + 1, Math.max(visible.length - 1, 0));
      this.highlight(visible);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      this.selected = Math.max(this.selected - 1, 0);
      this.highlight(visible);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const item = visible[this.selected];
      if (item) void this.runItem(item);
    } else if (event.key === "Escape") {
      event.preventDefault();
      this.close();
    }
  }

  private async runItem(item: PaletteCommand): Promise<void> {
    this.close();
    await item.run();
  }
}
