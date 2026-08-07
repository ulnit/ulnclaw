// Language switcher (P251) — dependency-free port of hermes
// apps/desktop `components/language-switcher.tsx`: a globe trigger that
// opens a searchable popover over LOCALE_OPTIONS. hermes disables cmdk's
// shouldFilter because its fuzzy reordering destroys the curated
// en → zh → zh-hant → ja → ar order; we keep the same semantics with a
// plain substring filter over the endonym, the (hidden) English name and
// the locale code, so "日本"/"japanese"/"ja" all find Japanese.

import { LOCALE_OPTIONS, currentLocale, onLocaleChange, setLocale, t } from "./i18n";
import type { Locale } from "./i18n";

function normalize(query: string): string {
  return query.trim().toLowerCase();
}

export class LanguageSwitcher {
  private popover: HTMLDivElement | null = null;
  private trigger: HTMLButtonElement;

  constructor(parent: HTMLElement) {
    this.trigger = document.createElement("button");
    this.trigger.id = "language-btn";
    this.trigger.className = "ghost language-btn";
    this.trigger.type = "button";
    this.trigger.title = t.language.switchTo;
    this.trigger.setAttribute("aria-label", t.language.switchTo);
    this.trigger.innerHTML = `<span aria-hidden="true">\u{1F310}</span><span class="lang-name"></span>`;
    this.renderTriggerLabel();
    this.trigger.onclick = () => this.toggle();
    parent.appendChild(this.trigger);
    onLocaleChange(() => {
      this.renderTriggerLabel();
      this.trigger.title = t.language.switchTo;
      this.trigger.setAttribute("aria-label", t.language.switchTo);
      if (this.popover) this.renderList(this.popover);
    });
    document.addEventListener("mousedown", (event) => {
      if (!this.popover) return;
      const target = event.target as Node;
      if (this.popover.contains(target) || this.trigger.contains(target)) return;
      this.close();
    });
  }

  private renderTriggerLabel(): void {
    const meta = LOCALE_OPTIONS.find((option) => option.id === currentLocale());
    const label = this.trigger.querySelector(".lang-name");
    if (label) label.textContent = meta?.name ?? "";
  }

  private toggle(): void {
    if (this.popover) {
      this.close();
      return;
    }
    const popover = document.createElement("div");
    popover.className = "language-pop";
    popover.innerHTML = `
      <input class="language-search" type="text" data-i18n-ph="language.searchPlaceholder" />
      <div class="language-list"></div>`;
    const search = popover.querySelector<HTMLInputElement>(".language-search")!;
    search.addEventListener("input", () => this.renderList(popover, search.value));
    popover.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        this.close();
      }
    });
    this.renderList(popover);
    document.body.appendChild(popover);
    const anchor = this.trigger.getBoundingClientRect();
    popover.style.left = `${Math.max(8, anchor.left)}px`;
    popover.style.bottom = `${Math.max(8, window.innerHeight - anchor.top + 6)}px`;
    this.popover = popover;
    search.focus();
  }

  private renderList(popover: HTMLDivElement, query = ""): void {
    const list = popover.querySelector(".language-list")!;
    list.innerHTML = "";
    const q = normalize(query);
    const filtered = LOCALE_OPTIONS.filter(
      (option) =>
        !q ||
        option.name.toLowerCase().includes(q) ||
        option.englishName.toLowerCase().includes(q) ||
        option.id.toLowerCase().includes(q),
    );
    if (!filtered.length) {
      const empty = document.createElement("div");
      empty.className = "language-empty";
      empty.textContent = t.language.noResults;
      list.appendChild(empty);
      return;
    }
    const active = currentLocale();
    for (const option of filtered) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "language-item" + (option.id === active ? " active" : "");
      const check = document.createElement("span");
      check.className = "language-check";
      check.textContent = option.id === active ? "✓" : "";
      const name = document.createElement("span");
      name.className = "language-name";
      name.textContent = option.name;
      const code = document.createElement("span");
      code.className = "language-code";
      code.textContent = option.id;
      row.append(check, name, code);
      row.onclick = () => {
        if (option.id !== active) setLocale(option.id as Locale);
        this.close();
      };
      list.appendChild(row);
    }
  }

  private close(): void {
    this.popover?.remove();
    this.popover = null;
  }
}
