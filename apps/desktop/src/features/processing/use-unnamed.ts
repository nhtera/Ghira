// SPDX-License-Identifier: Apache-2.0
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { adapter } from "./speakers-adapter";

/** Voices still unnamed in the meeting that just finished (empty until the adapter has any). */
export function useUnnamed(meeting: string | undefined) {
  const q = useQuery({
    queryKey: ["unnamed-speakers", meeting],
    enabled: !!meeting,
    queryFn: () => adapter.unnamed(meeting!),
  });
  const [named, setNamed] = useState<string[]>([]);
  const left = useMemo(
    () => (q.data ?? []).filter((s) => !named.includes(s.gid)),
    [q.data, named],
  );
  return {
    left,
    loaded: q.isSuccess,
    markNamed: (gid: string) => setNamed((n) => [...n, gid]),
  };
}
