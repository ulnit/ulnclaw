// Projects widget — first-class multi-folder workspaces (P163), backed by
// the gateway `/api/projects/*` API that shares the same per-profile
// `projects.db` as the `ulnclaw project` CLI. Projects anchor kanban
// worktrees (`kanban create --project`) and group desktop sessions.

import type { DiscoveredRepo, GatewayClient, Project } from "./gateway";

export class ProjectsWidget {
  private projects: Project[] = [];
  private activeId: string | null = null;
  private repos: DiscoveredRepo[] = [];
  private showArchived = false;

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  /** Build the static skeleton once; `refresh` fills it with data. */
  mount(): void {
    this.root.innerHTML = `
      <header id="projects-header">
        <span id="projects-active" class="projects-active"></span>
        <label class="projects-archived-toggle">
          <input id="projects-show-archived" type="checkbox" /> archived
        </label>
        <span class="spacer"></span>
        <button id="projects-scan" class="ghost" title="Scan the filesystem for git repos">Scan repos</button>
        <button id="projects-refresh" class="ghost" title="Refresh">↻</button>
        <button id="projects-new" class="primary">New project</button>
      </header>
      <div id="projects-list"></div>
      <section id="projects-repos-section">
        <h3>Discovered repos</h3>
        <div id="projects-repos"></div>
      </section>
      <dialog id="project-create">
        <form method="dialog">
          <h2>New project</h2>
          <label>Name
            <input id="project-create-name" type="text" placeholder="Hermes Agent" required />
          </label>
          <label>Folders (comma-separated; first = primary)
            <input id="project-create-folders" type="text" placeholder="~/code/hermes-agent" />
          </label>
          <label>Bind kanban board (optional slug)
            <input id="project-create-board" type="text" placeholder="default" />
          </label>
          <label class="check">
            <input id="project-create-use" type="checkbox" /> Set as active project
          </label>
          <menu>
            <button value="cancel">Cancel</button>
            <button id="project-create-save" value="save">Create</button>
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
    const [listing, repos] = await Promise.all([
      client.projectsList(this.showArchived),
      client.projectsRepos(),
    ]);
    this.projects = listing.projects;
    this.activeId = listing.active_id;
    this.repos = repos;
    this.render();
  }

  private async scan(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const rootsRaw = window.prompt(
      "Scan roots (comma-separated; empty = home directory):",
      "",
    );
    if (rootsRaw === null) return;
    const roots = rootsRaw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
    const button = this.root.querySelector("#projects-scan") as HTMLButtonElement;
    button.disabled = true;
    button.textContent = "Scanning…";
    try {
      const result = await client.projectScan(roots);
      if (result) {
        window.alert(`Recorded ${result.recorded} repo(s) into the discovery cache.`);
      }
      await this.refresh();
    } finally {
      button.disabled = false;
      button.textContent = "Scan repos";
    }
  }

  private async createProject(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const name = (this.root.querySelector("#project-create-name") as HTMLInputElement).value.trim();
    const foldersRaw = (this.root.querySelector("#project-create-folders") as HTMLInputElement).value;
    const board = (this.root.querySelector("#project-create-board") as HTMLInputElement).value.trim();
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
    });
    if (!created) {
      window.alert("Project creation failed (gateway unreachable or invalid input).");
      return;
    }
    (this.root.querySelector("#project-create-name") as HTMLInputElement).value = "";
    (this.root.querySelector("#project-create-folders") as HTMLInputElement).value = "";
    (this.root.querySelector("#project-create-board") as HTMLInputElement).value = "";
    await this.refresh();
  }

  private render(): void {
    const active = this.root.querySelector("#projects-active") as HTMLElement;
    const activeProject = this.projects.find((p) => p.id === this.activeId) || null;
    active.textContent = activeProject
      ? `Active: ${activeProject.name}`
      : "No active project";

    const list = this.root.querySelector("#projects-list") as HTMLElement;
    list.innerHTML = "";
    if (!this.projects.length) {
      list.innerHTML =
        '<p class="projects-empty">No projects yet — create one, or scan the filesystem for git repos.</p>';
    }
    for (const project of this.projects) {
      list.appendChild(this.renderCard(project));
    }

    const repos = this.root.querySelector("#projects-repos") as HTMLElement;
    repos.innerHTML = "";
    if (!this.repos.length) {
      repos.innerHTML =
        '<p class="projects-empty">Discovery cache is empty — run “Scan repos”.</p>';
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
    if (project.board_slug) {
      const board = document.createElement("span");
      board.className = "badge";
      board.textContent = `board: ${project.board_slug}`;
      header.appendChild(board);
    }
    if (project.archived) {
      const flag = document.createElement("span");
      flag.className = "badge";
      flag.textContent = "archived";
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
      star.title = folder.is_primary ? "Primary folder" : "Make primary";
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
      remove.title = "Remove folder";
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
      mk("Use", () => {
        if (!client) return;
        void client.projectSetActive(project.id).then(() => void this.refresh());
      });
    }
    mk("Add folder", () => {
      if (!client) return;
      const path = window.prompt("Folder path:");
      if (!path || !path.trim()) return;
      void client.projectAddFolder(project.id, path.trim()).then(() => void this.refresh());
    });
    mk(project.board_slug ? "Rebind board" : "Bind board", () => {
      if (!client) return;
      const board = window.prompt("Board slug (empty unbinds):", project.board_slug || "");
      if (board === null) return;
      void client
        .projectUpdate(project.id, { board_slug: board.trim() })
        .then(() => void this.refresh());
    });
    mk(project.archived ? "Restore" : "Archive", () => {
      if (!client) return;
      void client.projectArchive(project.id, project.archived).then(() => void this.refresh());
    });
    mk("Delete", () => {
      if (!client) return;
      if (!window.confirm(`Delete project "${project.name}"? This only removes the registry entry.`)) {
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
    adopt.title = "Create a project from this repo";
    adopt.textContent = "→ project";
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
