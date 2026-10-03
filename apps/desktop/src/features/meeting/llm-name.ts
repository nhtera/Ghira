// SPDX-License-Identifier: Apache-2.0
// The installed notes model's id, for "Answered on this Mac · <model>" and the
// detail header's engine pill.
import { useQuery } from "@tanstack/react-query";
import { ipc } from "../../ipc";

export function useLlmName(): string | null {
  const q = useQuery({
    queryKey: ["models-status"],
    queryFn: async () => {
      const r = await ipc.commands.modelsStatus();
      return r.status === "ok" ? r.data : null;
    },
  });
  return q.data?.models.find((m) => m.role === "llm" && m.installed)?.id ?? null;
}
