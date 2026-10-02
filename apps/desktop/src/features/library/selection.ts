// SPDX-License-Identifier: Apache-2.0
// Multi-select helpers: shift-click selects the run between the anchor and the
// clicked row (in the order shown), the way Finder and Mail do.
export function rangeIds(order: readonly string[], anchor: string | null, target: string): string[] {
  const to = order.indexOf(target);
  const from = anchor == null ? -1 : order.indexOf(anchor);
  if (to < 0) return [];
  if (from < 0) return [target];
  const [a, b] = from < to ? [from, to] : [to, from];
  return order.slice(a, b + 1);
}

export function toggleSelected(selected: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (!next.delete(id)) next.add(id);
  return next;
}

/** Shift-click: add the run to the selection (it never un-selects the rest). */
export const addRange = (selected: ReadonlySet<string>, ids: readonly string[]): Set<string> => new Set([...selected, ...ids]);
