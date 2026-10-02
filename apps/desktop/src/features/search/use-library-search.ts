// SPDX-License-Identifier: Apache-2.0
// Library search: debounced `searchMeetings`, 50 hits a page, "More results"
// appends the next page. The previous results stay while the next load.
import { keepPreviousData, useInfiniteQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { SearchHitView } from "../../bindings";
import { ipc } from "../../ipc";
import { searchParams, type LibraryFilters } from "../library/filters";

export const SEARCH_LIMIT = 50;
export const SEARCH_DEBOUNCE_MS = 200;

export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setV(value), ms);
    return () => clearTimeout(id);
  }, [value, ms]);
  return v;
}

export function useLibrarySearch(text: string, filters: LibraryFilters) {
  const typed = text.trim();
  const debounced = useDebounced(typed, SEARCH_DEBOUNCE_MS);
  const params = useMemo(() => searchParams(filters, new Date()), [filters]);
  const q = useInfiniteQuery({
    queryKey: ["search", debounced, params],
    enabled: debounced.length > 0,
    placeholderData: keepPreviousData,
    initialPageParam: 0,
    queryFn: async ({ pageParam }) => {
      const r = await ipc.commands.searchMeetings({
        text: debounced,
        ...params,
        meeting: null,
        limit: SEARCH_LIMIT,
        offset: pageParam,
      });
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
    getNextPageParam: (last, all) => (last.hits.length >= SEARCH_LIMIT ? all.length * SEARCH_LIMIT : undefined),
  });
  const hits = useMemo<SearchHitView[]>(() => q.data?.pages.flatMap((p) => p.hits) ?? [], [q.data]);
  return {
    /** The field has text (the list gives way to results). */
    active: typed.length > 0,
    query: debounced,
    hits,
    loading: typed !== debounced || (q.isFetching && !q.isFetchingNextPage),
    error: q.error?.message ?? null,
    hasMore: q.hasNextPage,
    loadingMore: q.isFetchingNextPage,
    more: () => void q.fetchNextPage(),
  };
}
