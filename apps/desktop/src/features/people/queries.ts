// SPDX-License-Identifier: Apache-2.0
// People reads: the list, one person, and the voice state. Mutations
// invalidate these (the keys share the "people" / "voice" prefixes).
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import type { PeopleList, PersonDetail, VoiceStatus } from "../../bindings";
import { ipc } from "../../ipc";
import { MEETINGS_KEY } from "../library/use-meetings";

const unwrap = async <T>(p: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>): Promise<T> => {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
};

export const peopleKey = ["people", "list"] as const;
export const personKey = (gid: string) => ["people", "person", gid] as const;
export const voiceKey = ["voice", "status"] as const;

export const usePeople = () => useQuery({ queryKey: peopleKey, queryFn: (): Promise<PeopleList> => unwrap(ipc.commands.listPeople()) });

export const usePersonDetail = (gid: string | null) =>
  useQuery({ queryKey: personKey(gid ?? ""), enabled: gid != null, queryFn: (): Promise<PersonDetail> => unwrap(ipc.commands.personDetail(gid!)) });

export const useVoiceStatus = (opts: { pollWhileNotReady?: boolean } = {}) =>
  useQuery({
    queryKey: voiceKey,
    queryFn: (): Promise<VoiceStatus> => unwrap(ipc.commands.voiceStatus()),
    // The voice model may still be downloading (onboarding): look again until it is there.
    refetchInterval: (q) => (opts.pollWhileNotReady && q.state.data && !q.state.data.modelReady ? 3000 : false),
  });

/**
 * After a change to people or voice data: reload what shows them. Names also
 * live in meetings (detail, library rows, search hits, name suggestions), so
 * those reload too.
 */
export function useInvalidatePeople() {
  const client = useQueryClient();
  return useCallback(() => {
    for (const queryKey of [["people"], ["voice"], ["meeting"], MEETINGS_KEY, ["knownSpeakerNames"], ["search"], ["related"]] as const) {
      void client.invalidateQueries({ queryKey });
    }
  }, [client]);
}
