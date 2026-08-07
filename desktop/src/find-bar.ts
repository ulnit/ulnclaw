// Desktop find-in-page bar — GUI twin of hermes' find-bar.tsx adapted
// from Electron's webContents.findInPage to a DOM implementation for
// the Tauri webview: Ctrl/Cmd+F opens, Enter/Shift+Enter and
// Ctrl/Cmd+G / Ctrl/Cmd+Shift+G step, Esc closes and clears.

import { t } from "./i18n";

export class FindBar {
  private bar: HTMLDivElement;
  private input: HTMLInputElement;
  private counter: HTMLSpanElement;
  private hits: HTMLElement[] = [];
  private current = 0;
  private active = false;

  constructor(
    private chatPane: HTMLElement,
    private messages: HTMLElement,
    private isVisible: () => boolean,
  ) {
    this.bar = document.createElement("div");
    this.bar.className = "find-bar";
    this.bar.hidden = true;

    this.input = document.createElement("input");
    this.input.type = "text";
    this.input.placeholder = t.find.placeholder;
    this.input.addEventListener("input", () => this.runQuery(this.input.value));
    this.input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        if (event.shiftKey) this.step(-1);
        else this.step(1);
      } else if (event.key === "Escape") {
        event.preventDefault();
        this.close();
      }
    });

    const prev = document.createElement("button");
    prev.className = "ghost find-step";
    prev.textContent = "\u{2191}";
    prev.title = t.find.prevTitle;
    prev.onclick = () => this.step(-1);

    const next = document.createElement("button");
    next.className = "ghost find-step";
    next.textContent = "\u{2193}";
    next.title = t.find.nextTitle;
    next.onclick = () => this.step(1);

    this.counter = document.createElement("span");
    this.counter.className = "find-count";
    this.counter.textContent = "0/0";

    const close = document.createElement("button");
    close.className = "ghost find-step";
    close.textContent = "\u{2715}";
    close.title = t.find.closeTitle;
    close.onclick = () => this.close();

    this.bar.append(this.input, prev, next, this.counter, close);
    this.chatPane.appendChild(this.bar);

    document.addEventListener("keydown", (event) => this.onKey(event));
  }

  /** Re-apply translated chrome after a locale switch (P251). */
  rerender(): void {
    this.input.placeholder = t.find.placeholder;
    const [prev, next, close] = Array.from(this.bar.querySelectorAll("button"));
    if (prev) prev.title = t.find.prevTitle;
    if (next) next.title = t.find.nextTitle;
    if (close) close.title = t.find.closeTitle;
  }

  isOpen(): boolean {
    return this.active;
  }

  open(): void {
    this.active = true;
    this.bar.hidden = false;
    this.input.focus();
    this.input.select();
  }

  close(): void {
    this.active = false;
    this.bar.hidden = true;
    this.input.value = "";
    this.clearHighlights();
    this.counter.textContent = "0/0";
  }

  /** Re-run the current query after the transcript re-renders. */
  refresh(): void {
    if (!this.active) return;
    this.runQuery(this.input.value);
  }

  private onKey(event: KeyboardEvent): void {
    const mod = event.ctrlKey || event.metaKey;
    if (mod && (event.key === "f" || event.key === "F")) {
      if (!this.isVisible()) return;
      event.preventDefault();
      this.open();
      return;
    }
    if (!this.active) return;
    if (mod && (event.key === "g" || event.key === "G")) {
      event.preventDefault();
      this.step(event.shiftKey ? -1 : 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      this.close();
    }
  }

  private step(direction: 1 | -1): void {
    if (this.hits.length === 0) return;
    this.hits[this.current]?.classList.remove("current");
    this.current = (this.current + direction + this.hits.length) % this.hits.length;
    const hit = this.hits[this.current];
    hit.classList.add("current");
    hit.scrollIntoView({ block: "center", behavior: "smooth" });
    this.renderCount();
  }

  private renderCount(): void {
    this.counter.textContent = this.hits.length
      ? `${this.current + 1}/${this.hits.length}`
      : this.input.value
        ? "0/0"
        : "0/0";
  }

  private runQuery(query: string): void {
    this.clearHighlights();
    const q = query.trim().toLowerCase();
    if (!q) {
      this.hits = [];
      this.current = 0;
      this.renderCount();
      return;
    }
    this.hits = this.highlight(this.messages, q);
    this.current = 0;
    if (this.hits.length > 0) {
      this.hits[0].classList.add("current");
      this.hits[0].scrollIntoView({ block: "center", behavior: "smooth" });
    }
    this.renderCount();
  }

  private highlight(root: HTMLElement, q: string): HTMLElement[] {
    const found: HTMLElement[] = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const parent = node.parentElement;
        if (!parent || parent.closest(".find-bar")) return NodeFilter.FILTER_REJECT;
        return node.nodeValue && node.nodeValue.toLowerCase().includes(q)
          ? NodeFilter.FILTER_ACCEPT
          : NodeFilter.FILTER_REJECT;
      },
    });
    const nodes: Text[] = [];
    while (walker.nextNode()) nodes.push(walker.currentNode as Text);
    for (const node of nodes) {
      const text = node.nodeValue || "";
      const lower = text.toLowerCase();
      const frag = document.createDocumentFragment();
      let pos = 0;
      let idx = lower.indexOf(q);
      while (idx !== -1) {
        frag.appendChild(document.createTextNode(text.slice(pos, idx)));
        const mark = document.createElement("mark");
        mark.className = "find-hit";
        mark.textContent = text.slice(idx, idx + q.length);
        frag.appendChild(mark);
        found.push(mark);
        pos = idx + q.length;
        idx = lower.indexOf(q, pos);
      }
      frag.appendChild(document.createTextNode(text.slice(pos)));
      node.parentNode?.replaceChild(frag, node);
    }
    return found;
  }

  private clearHighlights(): void {
    for (const mark of Array.from(this.messages.querySelectorAll("mark.find-hit"))) {
      const parent = mark.parentNode;
      if (!parent) continue;
      parent.replaceChild(document.createTextNode(mark.textContent || ""), mark);
      parent.normalize();
    }
    this.hits = [];
  }
}
