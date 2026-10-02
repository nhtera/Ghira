// SPDX-License-Identifier: Apache-2.0
// "Related by meaning": passages close in meaning to the search text, one per
// meeting. Slower than keywords, so it waits longer and needs 3+ characters.
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { RelatedHit } from "../../bindings";
import { ipc } from "../../ipc";
import { searchParams, type LibraryFilters } from "../library/filters";
import { useDebounced } from "./use-library-search";

export const RELATED_DEBOUNCE_MS = 600;
export const RELATED_MIN_CHARS = 3;
export const RELATED_LIMIT = 5;

export function useRelated(text: string, filters: LibraryFilters): RelatedHit[] {
  const debounced = useDebounced(text.trim(), RELATED_DEBOUNCE_MS);
  const fromMs = useMemo(() => searchParams(filters, new Date()).fromMs, [filters]);
  const q = useQuery({
    queryKey: ["related", debounced, fromMs],
    enabled: [...debounced].length >= RELATED_MIN_CHARS,
    queryFn: async () => {
      const r = await ipc.commands.relatedMeetings(debounced, { meetings: [], fromMs, toMs: null, persons: [] }, RELATED_LIMIT);
      return r.status === "ok" ? r.data : [];
    },
  });
  // Stale hits for an older text never show once the field changed.
  return debounced === text.trim() ? (q.data ?? []) : [];
}
