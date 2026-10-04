// SPDX-License-Identifier: Apache-2.0
// Where "Export…" saves: the remembered folder's name (never its path) and a
// way to pick another.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "../../ipc";

const KEY = ["export-destination"] as const;

export function useExportDestination(enabled: boolean, dialogTitle: string) {
  const client = useQueryClient();
  const q = useQuery({
    queryKey: KEY,
    enabled,
    queryFn: async () => {
      const r = await ipc.commands.exportDestination();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  /** Opens the folder dialog; resolves to an error message, or null. */
  const change = async (): Promise<string | null> => {
    const r = await ipc.commands.chooseExportFolder(dialogTitle);
    if (r.status === "error") return r.error;
    if (r.data != null) client.setQueryData(KEY, r.data);
    return null;
  };
  return { folder: q.data ?? null, change, failed: q.isError, loading: q.isPending, retry: () => void q.refetch() };
}

const VAULT_KEY = ["obsidian-vault"] as const;

/** The Obsidian vault folder's name (never its path) and a way to pick another. */
export function useObsidianVault(dialogTitle: string) {
  const client = useQueryClient();
  const q = useQuery({
    queryKey: VAULT_KEY,
    queryFn: async () => {
      const r = await ipc.commands.obsidianVault();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  const change = async (): Promise<string | null> => {
    const r = await ipc.commands.chooseObsidianVault(dialogTitle);
    if (r.status === "error") return r.error;
    if (r.data != null) client.setQueryData(VAULT_KEY, r.data);
    return null;
  };
  return { folder: q.data ?? null, change, failed: q.isError, loading: q.isPending, retry: () => void q.refetch() };
}

/** After a save the folder may have been chosen in the save dialog: ask again. */
export const refreshExportDestination = (client: ReturnType<typeof useQueryClient>) => client.invalidateQueries({ queryKey: KEY });
