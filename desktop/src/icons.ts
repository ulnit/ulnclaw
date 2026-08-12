// Central inline-SVG icon set (v0.3.1). The chrome no longer leans on emoji
// or rare unicode glyphs — those render inconsistently across platforms
// (color emoji on Windows clash with the monochrome design system). Every
// button/badge glyph is a stroke icon that inherits `currentColor`.

const s16 = (body: string): string =>
  `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

const s24 = (body: string): string =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const ICON = {
  clock: s16('<circle cx="8" cy="8" r="5.5"/><path d="M8 5.2v2.8l2 1.5"/>'),
  arrowUp: s16('<path d="M8 13V3"/><path d="M4.5 6.5 8 3l3.5 3.5"/>'),
  folder: s16('<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.8l1.4 1.8h4.8A1.5 1.5 0 0 1 14 6.3v6.2a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5Z"/>'),
  folderPlus: s16('<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.8l1.4 1.8h4.8A1.5 1.5 0 0 1 14 6.3v6.2a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5Z"/><path d="M8 7.6v4M6 9.6h4"/>'),
  gitBranch: s24('<line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>'),
  pencil: s16('<path d="m9.7 3.6 2.7 2.7L5.6 13l-3.1.5L3 10.4Z"/><path d="m8.6 4.7 2.7 2.7"/>'),
  download: s16('<path d="M8 2.5v7.5m0 0 3-3m-3 3-3-3"/><path d="M3 13h10"/>'),
  archive: s16('<rect x="2" y="3" width="12" height="3" rx="1"/><path d="M3.5 6v6A1.5 1.5 0 0 0 5 13.5h6a1.5 1.5 0 0 0 1.5-1.5V6"/><path d="M6.5 8.5h3"/>'),
  unarchive: s16('<path d="M3 8.5a5 5 0 1 0 1.6-3.7"/><path d="M3 2.8v2.4h2.4"/>'),
  trash: s16('<path d="M3 4.2h10"/><path d="M6.3 4V2.8h3.4V4"/><path d="M4.4 4.2 5 13.2h6l.6-9"/><path d="M6.6 6.5v4.5M9.4 6.5v4.5"/>'),
  x: s16('<path d="m4.5 4.5 7 7M11.5 4.5l-7 7"/>'),
  play: s16('<path d="M5.5 3.5v9l7-4.5Z"/>'),
  pause: s16('<path d="M5.8 3.5v9M10.2 3.5v9"/>'),
  gear: s16('<circle cx="8" cy="8" r="2.1"/><path d="M8 2.6v1.9M8 11.5v1.9M2.6 8h1.9M11.5 8h1.9M4.2 4.2l1.3 1.3M10.5 10.5l1.3 1.3M11.8 4.2l-1.3 1.3M5.5 10.5l-1.3 1.3"/>'),
  fork: s16('<circle cx="4.5" cy="3.8" r="1.6"/><circle cx="4.5" cy="12.2" r="1.6"/><circle cx="11.5" cy="3.8" r="1.6"/><path d="M4.5 5.4v5.2"/><path d="M11.5 5.4a3.5 3.5 0 0 1-3.5 3.5H6.2"/>'),
  pin: s24('<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 14 10.76V6a1 1 0 0 1 1-1 2 2 0 0 0 0-4H9a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>'),
  pinFilled: s24('<path d="M12 17v5"/><path fill="currentColor" d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 14 10.76V6a1 1 0 0 1 1-1 2 2 0 0 0 0-4H9a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>'),
  image: s16('<rect x="2.5" y="3" width="11" height="10" rx="1.5"/><circle cx="6" cy="6.5" r="1.2"/><path d="m3.5 11.5 3-3 2.5 2.5 2-2 1.5 1.5"/>'),
  file: s16('<path d="M4 2.5h5L12.5 6v7.5h-8.5Z"/><path d="M9 2.5V6h3.5"/>'),
  link: s16('<path d="M6.5 9.5 9.5 6.5"/><path d="M7.5 11 6 12.5a2.1 2.1 0 0 1-3-3L4.5 8"/><path d="M8.5 5 10 3.5a2.1 2.1 0 0 1 3 3L11.5 8"/>'),
  brain: s24('<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/>'),
  puzzle: s24('<M19.439 7.85c-.049.322.059.648.289.878l1.568 1.568c.47.47.706 1.087.706 1.704s-.235 1.233-.706 1.704l-1.611 1.611a.98.98 0 0 1-.837.276c-.47-.07-.802-.48-.968-.925a2.501 2.501 0 1 0-3.214 3.214c.446.166.855.497.925.968a.979.979 0 0 1-.276.837l-1.61 1.61a2.404 2.404 0 0 1-1.705.707 2.402 2.402 0 0 1-1.704-.706l-1.568-1.568a1.026 1.026 0 0 0-.877-.29c-.493.074-.84.5-1.02.968a2.5 2.5 0 1 1-3.233-3.233c.468-.18.894-.527.967-1.02a1.026 1.026 0 0 0-.289-.877l-1.568-1.568A2.402 2.402 0 0 1 1.998 12c0-.617.236-1.234.706-1.704L4.23 8.77c.24-.24.587-.353.925-.303.47.07.802.48.968.925a2.501 2.501 0 1 0 3.214-3.214c-.166-.446-.497-.855-.925-.968a.979.979 0 0 1 .276-.837l1.61-1.61a2.404 2.404 0 0 1 1.705-.707c.618 0 1.234.236 1.704.707l1.568 1.568c.23.23.556.338.877.29.493-.074.84-.5 1.02-.968a2.5 2.5 0 1 1 3.233 3.233c-.468.18-.894.527-.967 1.02Z'.replace('<M', '<path d="M') + '"/>'),
  wrench: s24('<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>'),
  warn: s16('<path d="M8 2.8 14.2 13.4H1.8Z"/><path d="M8 7v3.2M8 12v.01"/>'),
  info: s24('<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>'),
  cpu: s16('<rect x="4.5" y="4.5" width="7" height="7" rx="1.2"/><rect x="6.8" y="6.8" width="2.4" height="2.4" rx="0.5"/><path d="M6.2 4.5V2.5M9.8 4.5V2.5M6.2 13.5v-2M9.8 13.5v-2M4.5 6.2h-2M4.5 9.8h-2M13.5 6.2h-2M13.5 9.8h-2"/>'),
  chevronDown: s16('<path d="m4 6.2 4 4 4-4"/>'),
  check: s16('<path d="m3.5 8.6 3 3 6-6.8"/>'),
  checkCircle: s24('<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>'),
  ban: s24('<circle cx="12" cy="12" r="10"/><path d="m5.5 5.5 13 13"/>'),
  rotate: s24('<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>'),
  sparkle: s24('<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>'),
  shield: s24('<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>'),
  zap: s24('<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>'),
  copy: s24('<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>'),
  list: s24('<line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>'),
  eye: s24('<path d="M2.06 12.35a1 1 0 0 1 0-.7C3.42 8.1 7.26 5 12 5s8.58 3.1 9.94 6.65a1 1 0 0 1 0 .7C20.58 15.9 16.74 19 12 19s-8.58-3.1-9.94-6.65Z"/><circle cx="12" cy="12" r="3"/>'),
  keyboard: s24('<rect width="20" height="12" x="2" y="6" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h12"/>'),
} as const;

export type IconName = keyof typeof ICON;

/** Icon markup with an optional extra class (for tint hooks like g-warn). */
export function icon(name: IconName, cls?: string): string {
  const svg = ICON[name];
  return cls ? svg.replace("<svg ", `<svg class="${cls}" `) : svg;
}
