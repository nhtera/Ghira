// SPDX-License-Identifier: Apache-2.0
// Reads of one stored meeting (D6) for every part of the detail screen:
// header, notes, transcript, waveform. Refetched when the core says the
// meeting's state, job or notes changed; edits invalidate what they touched.
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { MeetingDetail, MeetingNotes, MeetingTranscript, TemplateInfo, Waveform } from "../bindings";
import { ipc } from "../ipc";

export const meetingKeys = {
  all: (id: string) => ["meeting", id] as const,
  detail: (id: string) => ["meeting", id, "detail"] as const,
  notes: (id: string) => ["meeting", id, "notes"] as const,
  transcript: (id: string) => ["meeting", id, "transcript"] as const,
  waveform: (id: string) => ["meeting", id, "waveform"] as const,
};

const unwrap = async <T>(p: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>): Promise<T> => {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
};

/** Everything about the meeting is read again (after an edit or a job). */
export const invalidateMeeting = (client: QueryClient, id: string) => client.invalidateQueries({ queryKey: meetingKeys.all(id) });

/** Keeps the open meeting's reads fresh while its jobs run. Mount once per screen. */
export function useMeetingEvents(id: string) {
  const client = useQueryClient();
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onCoreEvent((env) => {
        const e = env.event;
        if (!("meeting" in e) || e.meeting !== id) return;
        if (e.type === "notesReady" || e.type === "stateChanged") void invalidateMeeting(client, id);
        else if (e.type === "jobProgress") void client.invalidateQueries({ queryKey: meetingKeys.detail(id) });
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [client, id]);
}

export const useMeetingDetail = (id: string) =>
  useQuery({ queryKey: meetingKeys.detail(id), queryFn: (): Promise<MeetingDetail> => unwrap(ipc.commands.meetingDetail(id)) });

export const useMeetingNotes = (id: string) =>
  useQuery({ queryKey: meetingKeys.notes(id), queryFn: (): Promise<MeetingNotes> => unwrap(ipc.commands.meetingNotes(id)) });

export const useMeetingTranscript = (id: string) =>
  useQuery({ queryKey: meetingKeys.transcript(id), queryFn: (): Promise<MeetingTranscript> => unwrap(ipc.commands.meetingTranscript(id)) });

/** Computed once per meeting in Rust (a long meeting may take a few seconds the first time). */
export const useWaveform = (id: string, enabled = true) =>
  useQuery({ queryKey: meetingKeys.waveform(id), enabled, staleTime: Infinity, queryFn: (): Promise<Waveform> => unwrap(ipc.commands.waveformPeaks(id)) });

export const useTemplates = () =>
  useQuery({ queryKey: ["templates"], staleTime: Infinity, queryFn: (): Promise<TemplateInfo[]> => unwrap(ipc.commands.listTemplates()) });
