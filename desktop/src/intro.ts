// Chat intro (P255) — scoped port of hermes apps/desktop
// `chat/intro.tsx` + `intro-copy.jsonl`: a rotating headline/body pair
// shown on empty sessions so the blank canvas feels intentional.
// hermes keys copy pools by agent personality; the ulnclaw shell has
// no personality setting, so the neutral fallback pool is used,
// deterministic per session id (hermes seeds per render).

import { t } from "./i18n";

const POOL_SIZE = 5;

function hashSeed(seed: string): number {
  let hash = 5381;
  for (let i = 0; i < seed.length; i += 1) {
    hash = ((hash << 5) + hash + seed.charCodeAt(i)) >>> 0;
  }
  return hash;
}

export interface IntroCopy {
  body: string;
  headline: string;
}

export function introCopy(seed: string): IntroCopy {
  const index = (hashSeed(seed) % POOL_SIZE) + 1;
  const catalog = t.intro as unknown as Record<string, string>;
  return {
    body: catalog[`body${index}`],
    headline: catalog[`headline${index}`],
  };
}

/** Render the intro block into the messages pane (idempotent). */
export function renderIntro(messages: HTMLElement, seed: string): void {
  if (messages.querySelector(".chat-intro")) return;
  const copy = introCopy(seed);
  const node = document.createElement("div");
  node.className = "chat-intro";
  const headline = document.createElement("div");
  headline.className = "chat-intro-headline";
  headline.textContent = copy.headline;
  const body = document.createElement("div");
  body.className = "chat-intro-body";
  body.textContent = copy.body;
  node.append(headline, body);
  messages.appendChild(node);
}

export function clearIntro(messages: HTMLElement): void {
  messages.querySelector(".chat-intro")?.remove();
}
