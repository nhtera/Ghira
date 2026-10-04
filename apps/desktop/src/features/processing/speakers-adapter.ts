// SPDX-License-Identifier: Apache-2.0
// Post-meeting speaker commands, as the UI needs them.
import type { VoiceSuggestion } from "../../bindings";
import { ipc } from "../../ipc";

export type UnnamedSpeaker = {
  /** Store speaker gid. */
  gid: string;
  /** Shown as "Speaker N". */
  number: number;
  colorSlot: number;
  /** The sample span (meeting ms). */
  t0Ms: number | null;
  t1Ms: number | null;
  /** The final pass's voice match, until accepted or dismissed (only Me while third-party profiles are off). */
  suggestion: VoiceSuggestion | null;
};

export interface SpeakersAdapter {
  unnamed(meeting: string): Promise<UnnamedSpeaker[]>;
  rename(meeting: string, gid: string, name: string): Promise<{ ok: true } | { ok: false; error: string }>;
}

export const adapter: SpeakersAdapter = {
  unnamed: async (meeting) => {
    const r = await ipc.commands.meetingSpeakers(meeting);
    if (r.status === "error") return [];
    return r.data
      .filter((s) => s.name == null && !s.isMe && !s.notPerson)
      .map((s) => ({ gid: s.gid, number: s.number, colorSlot: s.colorSlot, t0Ms: s.sampleT0Ms, t1Ms: s.sampleT1Ms, suggestion: s.suggestion }));
  },
  rename: async (meeting, gid, name) => {
    const r = await ipc.commands.renameMeetingSpeaker(meeting, gid, name);
    return r.status === "ok" ? { ok: true } : { ok: false, error: r.error };
  },
};
