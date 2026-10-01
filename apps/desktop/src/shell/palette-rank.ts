// SPDX-License-Identifier: Apache-2.0
// ⌘K ranking (design rationale #5). Empty query: Go to, Actions, Recent.
// Typing: Actions (with "Ask '…'" first, so any query can still be asked),
// Meetings, Go to. Matching ignores case and Vietnamese accents ("chot"
// finds "chốt"), prefix matches first.
export type PaletteGroup = "goTo" | "actions" | "recent" | "meetings";

export type PaletteItem = {
  id: string;
  group: PaletteGroup;
  label: string;
  /** Extra words that should match (in both languages when useful). */
  keywords?: string[];
  hint?: string;
};

/** Lowercase without accents: "Chốt lịch Đà Nẵng" → "chot lich da nang". */
export function fold(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .replace(/đ/g, "d")
    .replace(/Đ/g, "d")
    .toLowerCase();
}

function score(item: PaletteItem, q: string): number {
  let best = -1;
  for (const text of [item.label, ...(item.keywords ?? [])]) {
    const f = fold(text);
    if (f.startsWith(q)) best = Math.max(best, 2);
    else if (f.split(/\s+/).some((w) => w.startsWith(q))) best = Math.max(best, 1);
    else if (f.includes(q)) best = Math.max(best, 0);
  }
  return best;
}

export type RankedGroup = { group: PaletteGroup; items: PaletteItem[] };

/**
 * `items`: everything the palette can offer (recent meetings carry group
 * "recent"; they are searched as "meetings" once the user types). `ask`
 * builds the "Ask '…'" item for a non-empty query.
 */
export function rankPalette(query: string, items: PaletteItem[], ask: (q: string) => PaletteItem): RankedGroup[] {
  const q = fold(query.trim());
  if (!q) {
    return (["goTo", "actions", "recent"] as const)
      .map((group) => ({ group, items: items.filter((i) => i.group === group) }))
      .filter((g) => g.items.length);
  }
  const ranked = (group: PaletteGroup, from: PaletteGroup[]) =>
    items
      .filter((i) => from.includes(i.group))
      .map((i, n) => ({ i: { ...i, group }, s: score(i, q), n }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s || a.n - b.n)
      .map((x) => x.i);
  return [
    { group: "actions" as const, items: [ask(query.trim()), ...ranked("actions", ["actions"])] },
    { group: "meetings" as const, items: ranked("meetings", ["recent", "meetings"]) },
    { group: "goTo" as const, items: ranked("goTo", ["goTo"]) },
  ].filter((g) => g.items.length);
}
