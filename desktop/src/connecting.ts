// Full-screen "CONNECTING" overlay (P249) — scoped port of hermes
// apps/desktop `gateway-connecting-overlay.tsx`: shown only during the
// initial cold boot while the gateway is unreachable, with the same
// exit choreography (text-out 360 ms → hold 300 ms → overlay fade
// 520 ms) and a reduced-motion fast path. Once the first healthy
// connection lands, the overlay never resurrects — later drops surface
// as toasts, not a modal wall.

const TEXT = "CONNECTING";
const TEXT_OUT_MS = 360;
const POST_TEXT_HOLD_MS = 300;
const OVERLAY_OUT_MS = 520;
const SCRAMBLE_CHARS = "▓▒░<>/\\|=#*+";

let overlay: HTMLElement | null = null;
let textEl: HTMLElement | null = null;
let scrambleTimer: number | null = null;
let coldBootDone = false;
let showing = false;

function prefersReducedMotion(): boolean {
  return Boolean(window.matchMedia?.("(prefers-reduced-motion: reduce)").matches);
}

function startScramble(): void {
  if (!textEl || prefersReducedMotion()) return;
  let resolved = 0;
  scrambleTimer = window.setInterval(() => {
    if (!textEl) return;
    resolved = Math.min(TEXT.length, resolved + 0.5);
    let out = "";
    for (let i = 0; i < TEXT.length; i += 1) {
      if (i < resolved) {
        out += TEXT[i];
      } else {
        out += SCRAMBLE_CHARS[Math.floor(Math.random() * SCRAMBLE_CHARS.length)];
      }
    }
    textEl.textContent = out;
    if (resolved >= TEXT.length && scrambleTimer !== null) {
      window.clearInterval(scrambleTimer);
      scrambleTimer = null;
    }
  }, 48);
}

/** Show the overlay — ignored once the cold boot has completed. */
export function showConnecting(): void {
  if (coldBootDone || showing) return;
  if (!overlay) mount();
  if (!overlay) return;
  showing = true;
  overlay.classList.remove("connecting-out", "connecting-text-out");
  overlay.hidden = false;
  if (textEl) textEl.textContent = prefersReducedMotion() ? TEXT : "";
  startScramble();
}

/** Exit choreography → unmount (hermes phase machine). */
export function hideConnecting(): void {
  if (!overlay || !showing) return;
  coldBootDone = true;
  showing = false;
  if (scrambleTimer !== null) {
    window.clearInterval(scrambleTimer);
    scrambleTimer = null;
  }
  const node = overlay;
  if (prefersReducedMotion()) {
    node.hidden = true;
    return;
  }
  node.classList.add("connecting-text-out");
  window.setTimeout(() => {
    node.classList.add("connecting-out");
    window.setTimeout(() => {
      node.hidden = true;
      node.classList.remove("connecting-out", "connecting-text-out");
    }, OVERLAY_OUT_MS);
  }, TEXT_OUT_MS + POST_TEXT_HOLD_MS);
}

function mount(): void {
  overlay = document.createElement("div");
  overlay.id = "connecting-overlay";
  overlay.hidden = true;
  textEl = document.createElement("div");
  textEl.className = "connecting-text";
  textEl.textContent = TEXT;
  const sub = document.createElement("div");
  sub.className = "connecting-sub";
  sub.textContent = "starting the ulnclaw gateway…";
  overlay.append(textEl, sub);
  document.body.appendChild(overlay);
}
