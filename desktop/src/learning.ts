// Desktop learning view — GUI twin of hermes' "star map" (learning
// graph) page: renders GET /api/learning/graph (learned skills +
// memory chunks + relation edges) with search, node detail, edit, and
// archive/delete over /api/learning/node. Overlay dialog scoped port.

import { t } from "./i18n";
import type { GatewayClient, LearningGraphPayload, LearningGraphNode } from "./gateway";

export class LearningOverlay {
  private dialog: HTMLDialogElement;
  private body: HTMLDivElement;
  private graph: LearningGraphPayload | null = null;
  private query = "";
  private kindFilter: "all" | "skill" | "memory" = "all";

  constructor(private client: () => GatewayClient | null) {
    this.dialog = document.createElement("dialog");
    this.dialog.className = "learning-dialog";
    const header = document.createElement("div");
    header.className = "learning-header";
    const title = document.createElement("h2");
    title.textContent = t.learning.title;
    const sub = document.createElement("span");
    sub.className = "learning-sub";
    sub.textContent = t.learning.tagline;
    header.append(title, sub);
    this.body = document.createElement("div");
    this.body.className = "learning-body";
    this.dialog.append(header, this.body);
    document.body.appendChild(this.dialog);
  }

  async open(): Promise<void> {
    this.query = "";
    this.kindFilter = "all";
    this.graph = null;
    this.body.innerHTML = "";
    const loading = document.createElement("div");
    loading.className = "learning-loading";
    loading.textContent = t.learning.building;
    this.body.appendChild(loading);
    this.dialog.showModal();

    const client = this.client();
    if (!client) {
      loading.textContent = t.learning.notConnected;
      return;
    }
    try {
      this.graph = await client.learningGraph();
      this.render();
    } catch (err) {
      loading.textContent = `Failed to load learning graph: ${err}`;
    }
  }

  private render(): void {
    const graph = this.graph;
    if (!graph) return;
    this.body.innerHTML = "";

    // Stats strip.
    const stats = document.createElement("div");
    stats.className = "learning-stats";
    const statDefs: [string, number | undefined][] = [
      ["learned skills", graph.stats.learned_skills],
      ["memories", graph.stats.memory_nodes],
      ["links", graph.edges.length],
      ["agent-created", graph.stats.agent_created],
      ["used", graph.stats.used],
    ];
    for (const [label, value] of statDefs) {
      const cell = document.createElement("div");
      cell.className = "learning-stat";
      const num = document.createElement("div");
      num.className = "learning-stat-num";
      num.textContent = String(value ?? 0);
      const lab = document.createElement("div");
      lab.className = "learning-stat-label";
      lab.textContent = label;
      cell.append(num, lab);
      stats.appendChild(cell);
    }
    this.body.appendChild(stats);

    // Cluster chips.
    if (graph.clusters.length > 0) {
      const clusters = document.createElement("div");
      clusters.className = "learning-clusters";
      for (const cluster of graph.clusters) {
        const chip = document.createElement("span");
        chip.className = "learning-cluster";
        chip.textContent = `${cluster.category} · ${cluster.count}`;
        clusters.appendChild(chip);
      }
      this.body.appendChild(clusters);
    }

    // Toolbar: kind filter + search.
    const toolbar = document.createElement("div");
    toolbar.className = "learning-toolbar";
    for (const kind of ["all", "skill", "memory"] as const) {
      const btn = document.createElement("button");
      btn.className = "learning-filter" + (kind === this.kindFilter ? " active" : "");
      btn.textContent = kind;
      btn.onclick = () => {
        this.kindFilter = kind;
        this.render();
      };
      toolbar.appendChild(btn);
    }
    const search = document.createElement("input");
    search.className = "learning-search";
    search.placeholder = t.learning.searchPlaceholder;
    search.value = this.query;
    search.addEventListener("input", () => {
      this.query = search.value;
      this.renderList();
    });
    toolbar.appendChild(search);
    this.body.appendChild(toolbar);

    const list = document.createElement("div");
    list.className = "learning-list";
    this.body.appendChild(list);
    this.renderList();
  }

  private neighbors(id: string): string[] {
    const graph = this.graph;
    if (!graph) return [];
    const out: string[] = [];
    for (const edge of graph.edges) {
      if (edge.source === id) out.push(edge.target);
      if (edge.target === id) out.push(edge.source);
    }
    return out;
  }

  private renderList(): void {
    const graph = this.graph;
    const list = this.body.querySelector<HTMLElement>(".learning-list");
    if (!graph || !list) return;
    list.innerHTML = "";
    const query = this.query.trim().toLowerCase();
    const visible = graph.nodes
      .filter((node) => {
        if (this.kindFilter !== "all" && node.kind !== this.kindFilter) return false;
        if (!query) return true;
        return (
          node.label.toLowerCase().includes(query) ||
          node.category.toLowerCase().includes(query)
        );
      })
      .slice(0, 300);
    if (visible.length === 0) {
      const empty = document.createElement("div");
      empty.className = "learning-empty";
      empty.textContent =
        graph.nodes.length === 0
          ? "No learning yet — skills the agent creates or uses, and memories, appear here."
          : t.learning.noMatches;
      list.appendChild(empty);
      return;
    }
    for (const node of visible) {
      list.appendChild(this.row(node));
    }
  }

  private row(node: LearningGraphNode): HTMLElement {
    const row = document.createElement("div");
    row.className = "learning-row";
    const icon = document.createElement("span");
    icon.className = "learning-icon";
    icon.textContent = node.kind === "memory" ? "🧠" : "🧩";
    const main = document.createElement("div");
    main.className = "learning-main";
    const label = document.createElement("div");
    label.className = "learning-label";
    label.textContent = node.label;
    const meta = document.createElement("div");
    meta.className = "learning-meta";
    const bits: string[] = [node.category];
    if (node.kind === "skill") {
      if (node.useCount > 0) bits.push(`used ×${node.useCount}`);
      if (node.createdBy === "agent") bits.push("agent-created");
      if (node.pinned) bits.push("pinned");
      if (node.state !== "active") bits.push(node.state);
    }
    const links = this.neighbors(node.id);
    if (links.length > 0) bits.push(`${links.length} link(s)`);
    if (node.timestamp) bits.push(new Date(node.timestamp * 1000).toLocaleDateString());
    meta.textContent = bits.join(" · ");
    main.append(label, meta);
    row.append(icon, main);
    row.onclick = () => void this.openNode(node);
    return row;
  }

  private async openNode(node: LearningGraphNode): Promise<void> {
    const client = this.client();
    if (!client) return;
    const detailDialog = document.createElement("dialog");
    detailDialog.className = "learning-node-dialog";
    const head = document.createElement("div");
    head.className = "learning-node-head";
    const title = document.createElement("h3");
    title.textContent = node.label;
    const kind = document.createElement("span");
    kind.className = "learning-node-kind";
    kind.textContent = node.kind;
    head.append(title, kind);
    const area = document.createElement("textarea");
    area.className = "learning-node-content";
    area.spellcheck = false;
    const status = document.createElement("div");
    status.className = "learning-node-status";
    const actions = document.createElement("div");
    actions.className = "learning-node-actions";

    const saveBtn = document.createElement("button");
    saveBtn.textContent = t.learning.save;
    saveBtn.disabled = true;
    saveBtn.onclick = async () => {
      try {
        await client.editLearningNode(node.id, area.value);
        status.textContent = t.learning.saved;
        setTimeout(() => {
          detailDialog.close();
          void this.open();
        }, 500);
      } catch (err) {
        status.textContent = `Save failed: ${err}`;
      }
    };
    const deleteBtn = document.createElement("button");
    deleteBtn.className = "danger";
    deleteBtn.textContent = node.kind === "skill" ? t.learning.archive : t.learning.delete;
    deleteBtn.disabled = true;
    deleteBtn.onclick = async () => {
      const verb = node.kind === "skill" ? "archive" : "delete";
      if (!window.confirm(`${verb} "${node.label}"?`)) return;
      try {
        await client.deleteLearningNode(node.id);
        status.textContent = verb === "archive" ? t.learning.archived : t.learning.deleted;
        setTimeout(() => {
          detailDialog.close();
          void this.open();
        }, 500);
      } catch (err) {
        status.textContent = `Failed: ${err}`;
      }
    };
    const pinBtn = document.createElement("button");
    pinBtn.className = "ghost";
    pinBtn.textContent = node.pinned ? t.learning.unpin : t.learning.pin;
    pinBtn.disabled = true;
    pinBtn.onclick = async () => {
      try {
        await client.curatorAction(node.pinned ? "unpin" : "pin", node.id);
        status.textContent = node.pinned ? t.learning.unpinned : t.learning.pinnedDone;
        setTimeout(() => {
          detailDialog.close();
          void this.open();
        }, 500);
      } catch (err) {
        status.textContent = `Failed: ${err}`;
      }
    };
    const restoreBtn = document.createElement("button");
    restoreBtn.className = "ghost";
    restoreBtn.textContent = t.learning.restore;
    restoreBtn.disabled = true;
    restoreBtn.onclick = async () => {
      try {
        await client.curatorAction("restore", node.id);
        status.textContent = t.learning.restored;
        setTimeout(() => {
          detailDialog.close();
          void this.open();
        }, 500);
      } catch (err) {
        status.textContent = `Failed: ${err}`;
      }
    };
    const closeBtn = document.createElement("button");
    closeBtn.className = "ghost";
    closeBtn.textContent = t.learning.close;
    closeBtn.onclick = () => detailDialog.close();
    if (node.kind === "skill") {
      actions.append(pinBtn);
      if (node.state === "archived") actions.append(restoreBtn);
    }
    actions.append(saveBtn, deleteBtn, closeBtn);
    detailDialog.append(head, area, status, actions);
    this.dialog.appendChild(detailDialog);
    detailDialog.showModal();
    detailDialog.addEventListener("close", () => detailDialog.remove());

    area.textContent = t.learning.loading;
    try {
      const detail = await client.learningNode(node.id);
      area.textContent = detail.content;
      saveBtn.disabled = false;
      deleteBtn.disabled = false;
      if (node.kind === "skill") {
        pinBtn.disabled = false;
        if (node.state === "archived") restoreBtn.disabled = false;
      }
    } catch (err) {
      area.textContent = `Failed to load node: ${err}`;
    }
  }
}
