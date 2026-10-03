// SPDX-License-Identifier: Apache-2.0
// Library filters (D3): People, Source, Template and a date preset. They apply
// to the list on the client and, where the search request can say it, to the
// search too (people are always checked client-side on the hits' meetings).
import type { MeetingRow, SearchRequest } from "../../bindings";

export type DatePreset = "today" | "week" | "month";
export type SourceKind = "live" | "file";
export type LibraryFilters = {
  people: string[];
  source: SourceKind[];
  template: string[];
  date: DatePreset | null;
  /** One folder (gid); `""`: meetings in no folder; null: any. */
  folder: string | null;
  /** Meetings with any of these tags (gids). */
  tags: string[];
};

export const NO_FILTERS: LibraryFilters = {
  people: [],
  source: [],
  template: [],
  date: null,
  folder: null,
  tags: [],
};

export const hasFilters = (f: LibraryFilters) => f.people.length + f.source.length + f.template.length + f.tags.length > 0 || f.date != null || f.folder != null;

export const toggleIn = <T>(list: readonly T[], v: T): T[] => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);

/** Imported files vs everything recorded here (the store says `live` or `import`/`file`). */
export const sourceOf = (row: Pick<MeetingRow, "source">): SourceKind => (row.source === "import" || row.source === "file" ? "file" : "live");

/** Inclusive start (unix ms) of a preset; calendar days, like the day groups. */
export function dateFrom(preset: DatePreset, now: Date): number {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (preset === "today") return today;
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() - (preset === "week" ? 6 : 29)).getTime();
}

export function matchesFilters(row: MeetingRow, f: LibraryFilters, now: Date): boolean {
  if (f.source.length && !f.source.includes(sourceOf(row))) return false;
  if (f.template.length && !f.template.includes(row.template ?? "general")) return false;
  if (f.people.length && !row.people.some((p) => f.people.includes(p.name))) return false;
  if (f.folder != null && (row.folder ?? "") !== f.folder) return false;
  if (f.tags.length && !row.tags.some((x) => f.tags.includes(x.gid))) return false;
  if (f.date && (row.startedAt == null || row.startedAt < dateFrom(f.date, now))) return false;
  return true;
}

export const applyFilters = (rows: MeetingRow[], f: LibraryFilters, now: Date) => (hasFilters(f) ? rows.filter((r) => matchesFilters(r, f, now)) : rows);

/** What the store can narrow itself: one source, one template, the date range, the folder and the tags. */
export function searchParams(f: LibraryFilters, now: Date): Pick<SearchRequest, "source" | "template" | "fromMs" | "toMs" | "folder" | "tags"> {
  return {
    source: f.source.length === 1 ? f.source[0]! : null,
    template: f.template.length === 1 ? f.template[0]! : null,
    fromMs: f.date ? dateFrom(f.date, now) : null,
    toMs: null,
    folder: f.folder,
    tags: f.tags.length ? f.tags : null,
  };
}

/** People names seen in the rows, most common first. */
export function peopleOf(rows: MeetingRow[]): string[] {
  const n = new Map<string, number>();
  for (const r of rows) for (const p of r.people) n.set(p.name, (n.get(p.name) ?? 0) + 1);
  return [...n.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([name]) => name);
}
