// Model visibility store (P252) — port of hermes apps/desktop
// `store/model-visibility.ts` + `store/provider-collapse.ts`: the
// explicit `provider::model` key set shown in the model picker, the
// hide-all sentinel bookkeeping, the `-fast`/date-snapshot family
// collapsing, the featured/top-N default expansion, and the persisted
// provider collapse state. Storage keys are ulnclaw-namespaced.

import type { ModelOptionRow } from "./gateway";

const STORAGE_KEY = "ulncl…odels";
const COLLAPSE_KEY = "ulncl…ders";

/** Models shown per provider before the user customizes the list. */
export const DEFAULT_VISIBLE_PER_PROVIDER = 50;

/** Stable key for a provider/model pair (`::` avoids colliding with
 * model ids that contain a single colon, e.g. `model:tag`). */
export const modelVisibilityKey = (provider: string, model: string): string =>
  `${provider}::${model}`;

/** Sentinel stored when the user explicitly hides ALL models for a
 * provider — distinguishes "hid everything" from "never customized". */
export const emptyProviderSentinelKey = (provider: string): string =>
  modelVisibilityKey(provider, "");

export const isProviderSentinel = (key: string): boolean => key.endsWith("::");

/** A model and its optional `…-fast` sibling collapsed into one row. */
export interface ModelFamily {
  fastId: string | null;
  id: string;
}

/** Collapse base + `…-fast` variants into families; drop date-pinned
 * snapshots superseded by their rolling alias (hermes parity). */
export function collapseModelFamilies(models: readonly string[]): ModelFamily[] {
  const present = new Set(models);
  const families: ModelFamily[] = [];
  const consumed = new Set<string>();
  for (const model of models) {
    if (consumed.has(model)) continue;
    if (/-fast$/i.test(model) && present.has(model.replace(/-fast$/i, ""))) continue;
    if (/-\d{8}$/.test(model) && present.has(model.replace(/-\d{8}$/, ""))) continue;
    const fastId = `${model}-fast`;
    const hasFast = present.has(fastId);
    families.push({ fastId: hasFast ? fastId : null, id: model });
    consumed.add(model);
    if (hasFast) consumed.add(fastId);
  }
  return families;
}

function loadVisible(): Set<string> | null {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((x): x is string => typeof x === "string"))
      : null;
  } catch {
    return null;
  }
}

let visibleModels: Set<string> | null = loadVisible();

/** Explicit visible keys, or null when never customized. */
export function getVisibleModels(): Set<string> | null {
  return visibleModels;
}

/** Persist an explicit set, or null to clear customization entirely
 * (reverts the picker to the curated defaults). */
export function setVisibleModels(keys: Set<string> | null): void {
  visibleModels = keys ? new Set(keys) : null;
  try {
    if (visibleModels) localStorage.setItem(STORAGE_KEY, JSON.stringify([...visibleModels]));
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // storage unavailable — choice lasts for this run only
  }
}

function loadCollapsed(): string[] {
  try {
    const raw = localStorage.getItem(COLLAPSE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

let collapsedProviders: string[] = loadCollapsed();

export function getCollapsedProviders(): readonly string[] {
  return collapsedProviders;
}

/** Toggle a provider slug in/out of the collapsed set (persisted;
 * deliberately never pruned against the active catalog — hermes
 * keeps the presentation preference across catalog changes). */
export function toggleCollapsedProvider(slug: string): void {
  collapsedProviders = collapsedProviders.includes(slug)
    ? collapsedProviders.filter((entry) => entry !== slug)
    : [...collapsedProviders, slug];
  try {
    localStorage.setItem(COLLAPSE_KEY, JSON.stringify(collapsedProviders));
  } catch {
    // ignore
  }
}

/** Default-visible keys: featured shortlist when the gateway reports
 * one, else the top-N collapsed families per provider. */
function expandProviderDefaults(provider: ModelOptionRow, target: Set<string>): void {
  const families = collapseModelFamilies(provider.models ?? []);
  const featured = provider.featured_models ?? [];
  const defaults = featured.length
    ? families.filter((family) => featured.includes(family.id))
    : families.slice(0, DEFAULT_VISIBLE_PER_PROVIDER);
  for (const family of defaults) {
    target.add(modelVisibilityKey(provider.slug, family.id));
  }
}

export function defaultVisibleKeys(providers: readonly ModelOptionRow[]): Set<string> {
  const keys = new Set<string>();
  for (const provider of providers) expandProviderDefaults(provider, keys);
  return keys;
}

/** Canonical working set: stored keys plus default expansion for any
 * provider the user hasn't customized. Hide-all sentinels PRESERVED —
 * this is the set the toggle handlers mutate and persist. */
export function resolveVisibleKeys(
  stored: Set<string> | null,
  providers: readonly ModelOptionRow[],
): Set<string> {
  if (!stored) return defaultVisibleKeys(providers);
  if (stored.size === 0) return new Set();
  const next = new Set(stored);
  for (const provider of providers) {
    const prefix = `${provider.slug}::`;
    const hasStoredProvider = [...stored].some(
      (key) => key.startsWith(prefix) && !isProviderSentinel(key),
    );
    const hasSentinel = stored.has(emptyProviderSentinelKey(provider.slug));
    if (hasStoredProvider || hasSentinel) continue;
    expandProviderDefaults(provider, next);
  }
  return next;
}

/** Display set: resolved keys with sentinel bookkeeping stripped. */
export function effectiveVisibleKeys(
  stored: Set<string> | null,
  providers: readonly ModelOptionRow[],
): Set<string> {
  const next = resolveVisibleKeys(stored, providers);
  for (const key of [...next]) {
    if (isProviderSentinel(key)) next.delete(key);
  }
  return next;
}

/** Next persisted set after toggling one model row. Seeds from
 * resolveVisibleKeys so other providers' sentinels survive; toggling
 * off the last model records the hide-all sentinel, re-enabling a
 * model clears only that provider's sentinel (hermes semantics: you
 * get back exactly what you re-enable, not the curated defaults). */
export function toggleModelVisibility(
  stored: Set<string> | null,
  providers: readonly ModelOptionRow[],
  providerSlug: string,
  model: string,
): Set<string> {
  const next = resolveVisibleKeys(stored, providers);
  const key = modelVisibilityKey(providerSlug, model);
  const sentinel = emptyProviderSentinelKey(providerSlug);
  if (next.has(key)) {
    next.delete(key);
    const remaining = [...next].some(
      (k) => k.startsWith(`${providerSlug}::`) && !isProviderSentinel(k),
    );
    if (!remaining) next.add(sentinel);
  } else {
    next.delete(sentinel);
    next.add(key);
  }
  return next;
}

/** Next persisted set after flipping a provider master switch. */
export function setProviderVisibility(
  stored: Set<string> | null,
  providers: readonly ModelOptionRow[],
  providerSlug: string,
  visible: boolean,
): Set<string> {
  const next = resolveVisibleKeys(stored, providers);
  const sentinel = emptyProviderSentinelKey(providerSlug);
  const provider = providers.find((p) => p.slug === providerSlug);
  const families = collapseModelFamilies(provider?.models ?? []);
  for (const key of [...next]) {
    if (key.startsWith(`${providerSlug}::`)) next.delete(key);
  }
  if (visible) {
    for (const family of families) {
      next.add(modelVisibilityKey(providerSlug, family.id));
    }
    if (families.length === 0) next.delete(sentinel);
  } else {
    next.add(sentinel);
  }
  return next;
}
