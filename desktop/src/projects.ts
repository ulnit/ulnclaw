// Projects widget — first-class multi-folder workspaces (P163), backed by
// the gateway `/api/projects/*` API that shares the same per-profile
// `projects.db` as the `ulnclaw project` CLI. Projects anchor kanban
// worktrees (`kanban create --project`) and group desktop sessions.

import type { DiscoveredRepo, GatewayClient, Project, SessionRow } from "./gateway";
import { fmt, t } from "./i18n";

export class ProjectsWidget {
  private projects: Project[] = [];
  private activeId: string | null = null;
  private repos: DiscoveredRepo[] = [];
  private showArchived = false;
  /** P430: session census per project slug (from /api/sessions). */
  private sessionCounts = new Map<string, number>();

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
    // P427: start a chat session rooted at the project's folder.
    private newSessionInProject: ((cwd: string, title: string) => void) | null = null,
  ) {}

  /** Build the static skeleton once; `refresh` fills it with data. */
  mount(): void {
    this.root.innerHTML = `
      <header id="projects-header">
        <span id="projects-active" class="projects-active"></span>
        <label class="projects-archived-toggle">
          <input id="projects-show-archived" type="checkbox" /> <span data-i18n="projects.archived">archived</span>
        </label>
        <span class="spacer"></span>
        <button id="projects-scan" class="ghost" title="Scan the filesystem for git repos" data-i18n-title="projects.scanTitle" data-i18n="projects.scanRepos">Scan repos</button>
        <button id="projects-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
        <button id="projects-new" class="primary" data-i18n="projects.newProject">New project</button>
      </header>
      <div id="projects-list"></div>
      <section id="projects-repos-section">
        <h3 data-i18n="projects.discoveredRepos">Discovered repos</h3>
        <div id="projects-repos"></div>
      </section>
      <dialog id="project-create">
        <form method="dialog">
          <h2 data-i18n="projects.newProject">New project</h2>
          <label><span data-i18n="projects.nameLabel">Name</span>
            <input id="project-create-name" type="text" placeholder="Hermes Agent" required />
          </label>
          <label><span data-i18n="projects.foldersLabel">Folders (comma-separated; first = primary)</span>
            <input id="project-create-folders" type="text" placeholder="~/code/hermes-agent" />
          </label>
          <label><span data-i18n="projects.boardLabel">Bind kanban board (optional slug)</span>
            <input id="project-create-board" type="text" placeholder="default" />
          </label>
          <label><span data-i18n="projects.descriptionLabel">Description (optional)</span>
            <input id="project-create-description" type="text" />
          </label>
          <label><span data-i18n="projects.iconLabel">Icon emoji (optional)</span>
            <input id="project-create-icon" type="text" maxlength="8" />
          </label>
          <label class="check">
            <input id="project-create-use" type="checkbox" /> <span data-i18n="projects.setActive">Set as active project</span>
          </label>
          <menu>
            <button value="cancel" data-i18n="chrome.cancel">Cancel</button>
            <button id="project-create-save" value="save" data-i18n="projects.create">Create</button>
          </menu>
        </form>
      </dialog>`;

    (this.root.querySelector("#projects-refresh") as HTMLButtonElement).onclick = () =>
      void this.refresh();
    (this.root.querySelector("#projects-new") as HTMLButtonElement).onclick = () => {
      const dialog = this.root.querySelector("#project-create") as HTMLDialogElement;
      dialog.showModal();
    };
    (this.root.querySelector("#projects-scan") as HTMLButtonElement).onclick = () =>
      void this.scan();
    (this.root.querySelector("#projects-show-archived") as HTMLInputElement).onchange = (e) => {
      this.showArchived = (e.target as HTMLInputElement).checked;
      void this.refresh();
    };

    const dialog = this.root.querySelector("#project-create") as HTMLDialogElement;
    dialog.addEventListener("close", () => {
      if (dialog.returnValue !== "save") return;
      void this.createProject();
    });
  }

  /** Load data when the tab becomes visible. */
  start(): void {
    void this.refresh();
  }

  stop(): void {
    // No polling loop yet — nothing to stop.
  }

  private async refresh(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const [listing, repos, sessions] = await Promise.all([
      client.projectsList(this.showArchived),
      client.projectsRepos(),
      // P430: count sessions per owning-project slug; a gateway hiccup
      // must not break the projects view.
      client.listSessions().catch(() => [] as SessionRow[]),
    ]);
    this.projects = listing.projects;
    this.activeId = listing.active_id;
    this.repos = repos;
    const counts = new Map<string, number>();
    for (const session of sessions) {
      if (!session.project) continue;
      counts.set(session.project, (counts.get(session.project) ?? 0) + 1);
    }
    this.sessionCounts = counts;
    this.render();
  }

  private async scan(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const rootsRaw = window.prompt(t.projects.scanRootsPrompt, "");
    if (rootsRaw === null) return;
    const roots = rootsRaw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
    const button = this.root.querySelector("#projects-scan") as HTMLButtonElement;
    button.disabled = true;
    button.textContent = t.projects.scanning;
    try {
      const result = await client.projectScan(roots);
      if (result) {
        window.alert(fmt(t.projects.scanRecorded, { count: result.recorded }));
      }
      await this.refresh();
    } finally {
      button.disabled = false;
      button.textContent = t.projects.scanRepos;
    }
  }

  private async createProject(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const name = (this.root.querySelector("#project-create-name") as HTMLInputElement).value.trim();
    const foldersRaw = (this.root.querySelector("#project-create-folders") as HTMLInputElement).value;
    const board = (this.root.querySelector("#project-create-board") as HTMLInputElement).value.trim();
    const description = (this.root.querySelector("#project-create-description") as HTMLInputElement).value.trim();
    const icon = (this.root.querySelector("#project-create-icon") as HTMLInputElement).value.trim();
    const use = (this.root.querySelector("#project-create-use") as HTMLInputElement).checked;
    if (!name) return;
    const folders = foldersRaw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
    const created = await client.projectCreate({
      name,
      folders: folders.length ? folders : undefined,
      board_slug: board || undefined,
      use,
      description: description || undefined,
      icon: icon || undefined,
    });
    if (!created) {
      window.alert(t.projects.createFailed);
      return;
    }
    (this.root.querySelector("#project-create-name") as HTMLInputElement).value = "";
    (this.root.querySelector("#project-create-folders") as HTMLInputElement).value = "";
    (this.root.querySelector("#project-create-board") as HTMLInputElement).value = "";
    (this.root.querySelector("#project-create-description") as HTMLInputElement).value = "";
    (this.root.querySelector("#project-create-icon") as HTMLInputElement).value = "";
    await this.refresh();
  }

  /** Re-render cached data after a locale switch (P251). */
  rerender(): void {
    if (this.projects.length || this.repos.length) this.render();
  }

  private render(): void {
    const active = this.root.querySelector("#projects-active") as HTMLElement;
    const activeProject = this.projects.find((p) => p.id === this.activeId) || null;
    active.textContent = activeProject
      ? fmt(t.projects.activePrefix, { name: activeProject.name })
      : t.projects.noActiveProject;

    const list = this.root.querySelector("#projects-list") as HTMLElement;
    list.innerHTML = "";
    if (!this.projects.length) {
      list.innerHTML = '<p class="projects-empty"></p>';
      list.querySelector(".projects-empty")!.textContent = t.projects.empty;
    }
    for (const project of this.projects) {
      list.appendChild(this.renderCard(project));
    }

    const repos = this.root.querySelector("#projects-repos") as HTMLElement;
    repos.innerHTML = "";
    if (!this.repos.length) {
      repos.innerHTML = '<p class="projects-empty"></p>';
      repos.querySelector(".projects-empty")!.textContent = t.projects.reposEmpty;
    }
    for (const repo of this.repos) {
      repos.appendChild(this.renderRepo(repo));
    }
  }

  private renderCard(project: Project): HTMLElement {
    const client = this.client();
    const card = document.createElement("div");
    card.className = "project-card" + (project.archived ? " archived" : "");
    if (project.id === this.activeId) card.classList.add("active");

    const header = document.createElement("div");
    header.className = "project-card-header";
    const name = document.createElement("span");
    name.className = "project-name";
    name.textContent = `${project.icon || ""} ${project.name}`.trim();
    header.appendChild(name);
    const slug = document.createElement("span");
    slug.className = "project-slug";
    slug.textContent = project.slug;
    header.appendChild(slug);
    const census = this.sessionCounts.get(project.slug) ?? 0;
    if (census > 0) {
      const count = document.createElement("span");
      count.className = "badge project-session-count";
      count.textContent = fmt(t.projects.sessionCount, { count: String(census) });
      header.appendChild(count);
    }
    if (project.board_slug) {
      const board = document.createElement("span");
      board.className = "badge";
      board.textContent = fmt(t.projects.boardBadge, { slug: project.board_slug });
      header.appendChild(board);
    }
    if (project.archived) {
      const flag = document.createElement("span");
      flag.className = "badge";
      flag.textContent = t.projects.archived;
      header.appendChild(flag);
    }
    card.appendChild(header);

    if (project.description) {
      const about = document.createElement("p");
      about.className = "project-about";
      about.textContent = project.description;
      card.appendChild(about);
    }

    const folders = document.createElement("ul");
    folders.className = "project-folders";
    for (const folder of project.folders) {
      const item = document.createElement("li");
      const star = document.createElement("button");
      star.className = "icon";
      star.title = folder.is_primary ? t.projects.primaryFolder : t.projects.makePrimary;
      star.textContent = folder.is_primary ? "★" : "☆";
      if (!folder.is_primary) {
        star.onclick = () => {
          if (!client) return;
          void client.projectSetPrimary(project.id, folder.path).then(() => void this.refresh());
        };
      }
      item.appendChild(star);
      const path = document.createElement("span");
      path.className = "project-folder-path";
      path.textContent = folder.path + (folder.label ? ` (${folder.label})` : "");
      item.appendChild(path);
      const remove = document.createElement("button");
      remove.className = "icon danger";
      remove.title = t.projects.removeFolder;
      remove.textContent = "✕";
      remove.onclick = () => {
        if (!client) return;
        void client.projectRemoveFolder(project.id, folder.path).then(() => void this.refresh());
      };
      item.appendChild(remove);
      folders.appendChild(item);
    }
    card.appendChild(folders);

    const actions = document.createElement("div");
    actions.className = "project-actions";
    const mk = (label: string, onclick: () => void, danger = false): HTMLButtonElement => {
      const button = document.createElement("button");
      button.className = danger ? "ghost danger" : "ghost";
      button.textContent = label;
      button.onclick = onclick;
      actions.appendChild(button);
      return button;
    };
    if (project.id !== this.activeId) {
      mk(t.projects.use, () => {
        if (!client) return;
        void client.projectSetActive(project.id).then(() => void this.refresh());
      });
    }
    // P427: spin up a session bound to the project's cwd (primary
    // folder first, else the first registered folder).
    const sessionCwd = project.primary_path ?? project.folders[0]?.path;
    if (sessionCwd && this.newSessionInProject) {
      mk(t.projects.newSessionHere, () => this.newSessionInProject!(sessionCwd, project.name));
    }
    mk(t.projects.addFolder, () => {
      if (!client) return;
      const path = window.prompt(t.projects.folderPathPrompt);
      if (!path || !path.trim()) return;
      void client.projectAddFolder(project.id, path.trim()).then(() => void this.refresh());
    });
    mk(t.projects.rename, () => {
      if (!client) return;
      const next = window.prompt(t.projects.renamePrompt, project.name);
      if (next === null || !next.trim() || next.trim() === project.name) return;
      void client.projectUpdate(project.id, { name: next.trim() }).then(() => void this.refresh());
    });
    mk(t.projects.editAbout, () => {
      if (!client) return;
      const next = window.prompt(t.projects.aboutPrompt, project.description || "");
      if (next === null || next === (project.description || "")) return;
      void client
        .projectUpdate(project.id, { description: next.trim() })
        .then(() => void this.refresh());
    });
    mk(project.board_slug ? t.projects.rebindBoard : t.projects.bindBoard, () => {
      if (!client) return;
      const board = window.prompt(t.projects.boardSlugPrompt, project.board_slug || "");
      if (board === null) return;
      void client
        .projectUpdate(project.id, { board_slug: board.trim() })
        .then(() => void this.refresh());
    });
    mk(project.archived ? t.projects.restore : t.projects.archive, () => {
      if (!client) return;
      void client.projectArchive(project.id, project.archived).then(() => void this.refresh());
    });
    mk(t.projects.delete, () => {
      if (!client) return;
      if (!window.confirm(fmt(t.projects.deleteConfirm, { name: project.name }))) {
        return;
      }
      void client.projectDelete(project.id).then(() => void this.refresh());
    }, true);
    card.appendChild(actions);
    return card;
  }

  private renderRepo(repo: DiscoveredRepo): HTMLElement {
    const client = this.client();
    const row = document.createElement("div");
    row.className = "project-repo-row";
    const label = document.createElement("span");
    label.className = "project-repo-label";
    label.textContent = repo.label;
    row.appendChild(label);
    const root = document.createElement("span");
    root.className = "project-repo-root";
    root.textContent = repo.root;
    row.appendChild(root);
    const seen = document.createElement("span");
    seen.className = "project-repo-seen";
    seen.textContent = new Date(repo.last_seen * 1000).toLocaleDateString();
    row.appendChild(seen);
    const adopt = document.createElement("button");
    adopt.className = "ghost";
    adopt.title = t.projects.createFromRepo;
    adopt.textContent = t.projects.toProject;
    adopt.onclick = () => {
      if (!client) return;
      void client
        .projectCreate({ name: repo.label, folders: [repo.root] })
        .then(() => void this.refresh());
    };
    row.appendChild(adopt);
    return row;
  }
}
