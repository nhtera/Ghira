// SPDX-License-Identifier: Apache-2.0
// Recent searches: this phone's localStorage only, never the core or the network.
const KEY = "ghi.search.recent";
export const MAX_RECENT = 8;

export function loadRecent(): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v)
      ? v.filter((x): x is string => typeof x === "string").slice(0, MAX_RECENT)
      : [];
  } catch {
    return [];
  }
}

/** The list with `query` first (no repeat, newest first, capped). */
export function withRecent(list: string[], query: string): string[] {
  const q = query.trim();
  if (!q) return list;
  return [q, ...list.filter((x) => x.toLowerCase() !== q.toLowerCase())].slice(
    0,
    MAX_RECENT,
  );
}

export function saveRecent(list: string[]) {
  try {
    if (list.length) localStorage.setItem(KEY, JSON.stringify(list));
    else localStorage.removeItem(KEY);
  } catch {
    /* private mode or blocked storage: recents just don't persist */
  }
}

/** Dispatched on `window` when the recents are wiped (the screen forgets its copy). */
export const RECENT_CLEARED = "ghi-recent-cleared";

/** Forgets every recent search (Settings → delete everything calls this). */
export function clearRecentSearches() {
  saveRecent([]);
  window.dispatchEvent(new Event(RECENT_CLEARED));
}
