// SPDX-License-Identifier: Apache-2.0
// Search as you type: debounced, and an older answer never replaces a newer one.
import { useEffect, useRef, useState } from "react";
import type { SearchHitView } from "../../bindings";
import { ipc } from "../../ipc";

export const DEBOUNCE_MS = 200;
export const LIMIT = 50;

export type SearchState = {
  status: "idle" | "searching" | "done" | "error";
  hits: SearchHitView[];
  query: string;
};

const IDLE: SearchState = { status: "idle", hits: [], query: "" };

export function useSearch(text: string): SearchState {
  // The last answer and the query it answers.
  const [answer, setAnswer] = useState<SearchState>(IDLE);
  const latest = useRef(0);
  const query = text.trim();

  useEffect(() => {
    const n = ++latest.current;
    if (!query) return;
    const timer = window.setTimeout(async () => {
      const r = await ipc.commands
        .searchMeetings({
          text: query,
          source: null,
          template: null,
          fromMs: null,
          toMs: null,
          meeting: null,
          limit: LIMIT,
          offset: 0,
        })
        .catch(() => null);
      if (n !== latest.current) return;
      setAnswer(
        r?.status === "ok"
          ? { status: "done", hits: r.data.hits, query }
          : { status: "error", hits: [], query },
      );
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query]);

  if (!query) return IDLE;
  // Until the answer for this query arrives, the previous hits stay on screen.
  return answer.query === query
    ? answer
    : { status: "searching", hits: answer.hits, query };
}
