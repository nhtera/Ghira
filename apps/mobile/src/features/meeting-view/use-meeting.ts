// SPDX-License-Identifier: Apache-2.0
// One meeting's data for M4: detail, notes, transcript and the sync chip,
// kept fresh from coreEvent (jobProgress, stateChanged), plus the edits (an
// action's done flag, a transcript line's text), which update the screen
// first and are undone by a reload if the core refuses.
import { useCallback, useEffect, useState } from "react";
import type {
  CoreEvent,
  MeetingChip,
  MeetingDetail,
  MeetingNotes,
  MeetingTranscript,
} from "../../bindings";
import { ipc } from "../../ipc";
import { UNLOCKED_EVENT } from "../app-lock/events";
import { useWindowEvent } from "./use-window-event";
import { classifyError, type ReadError } from "./read-error";
import { chipWithProgress } from "../meeting-list/use-meeting-list";

export type MeetingData = {
  /** `locked`, `starting`, `missing` and `failed` are why the core would not answer. */
  status: "loading" | "ready" | ReadError;
  detail?: MeetingDetail;
  notes: MeetingNotes;
  transcript: MeetingTranscript;
  chip?: MeetingChip;
};

const NO_NOTES: MeetingNotes = {
  blocks: [],
  actionItems: [],
  sections: [],
  marks: [],
  linked: 0,
};
const NO_TRANSCRIPT: MeetingTranscript = {
  version: null,
  segments: [],
  marks: [],
  topics: [],
};

async function fetchMeeting(id: string): Promise<MeetingData> {
  const [detail, notes, transcript, chips] = await Promise.all([
    ipc.commands.meetingDetail(id).catch(() => null),
    ipc.commands.meetingNotes(id).catch(() => null),
    ipc.commands.meetingTranscript(id).catch(() => null),
    ipc.commands.meetingChips([id]).catch(() => null),
  ]);
  if (detail?.status !== "ok")
    return {
      status: detail ? classifyError(detail.error) : "failed",
      notes: NO_NOTES,
      transcript: NO_TRANSCRIPT,
    };
  return {
    status: "ready",
    detail: detail.data,
    notes: notes?.status === "ok" ? notes.data : NO_NOTES,
    transcript: transcript?.status === "ok" ? transcript.data : NO_TRANSCRIPT,
    chip: chips?.status === "ok" ? chips.data[0]?.chip : undefined,
  };
}

export function useMeeting(id: string) {
  // The screen is keyed by the meeting, so another meeting starts from "loading".
  const [data, setData] = useState<MeetingData>({
    status: "loading",
    notes: NO_NOTES,
    transcript: NO_TRANSCRIPT,
  });
  const [percent, setPercent] = useState<number>();
  const [version, setVersion] = useState(0);
  const load = useCallback(() => setVersion((v) => v + 1), []);

  useEffect(() => {
    let alive = true;
    let retry: number | undefined;
    void fetchMeeting(id).then((d) => {
      if (!alive) return;
      setData(d);
      // The core is still opening the store: ask again shortly.
      if (d.status === "starting") retry = window.setTimeout(load, 1000);
    });
    return () => {
      alive = false;
      window.clearTimeout(retry);
    };
  }, [id, version, load]);
  useWindowEvent(UNLOCKED_EVENT, load);

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    const onEvent = ({ event: e }: CoreEvent) => {
      if (e.type === "jobProgress" && e.meeting === id)
        setPercent(Math.round(Math.min(Math.max(e.progress ?? 0, 0), 1) * 100));
      else if (e.type === "stateChanged" && e.meeting === id) {
        setPercent(undefined);
        load();
      } else if (e.type === "notesReady" && e.meeting === id) load();
    };
    void ipc.onCoreEvent(onEvent).then((u) => {
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [id, load]);

  const setActionDone = useCallback(
    async (item: string, done: boolean) => {
      setData((d) => ({
        ...d,
        notes: {
          ...d.notes,
          actionItems: d.notes.actionItems.map((a) =>
            a.gid === item ? { ...a, done } : a,
          ),
        },
      }));
      const r = await ipc.commands
        .setActionDone(id, item, done)
        .catch(() => null);
      if (r?.status !== "ok") load();
    },
    [id, load],
  );

  /** Saves a corrected line; the words' timings no longer line up, so they are dropped. */
  const saveSegment = useCallback(
    async (segment: string, text: string) => {
      const r = await ipc.commands
        .updateSegmentText(id, segment, text)
        .catch(() => null);
      if (r?.status !== "ok") return false;
      setData((d) => ({
        ...d,
        transcript: {
          ...d.transcript,
          segments: d.transcript.segments.map((s) =>
            s.gid === segment ? { ...s, text, edited: true, words: [] } : s,
          ),
        },
      }));
      return true;
    },
    [id],
  );

  /** "Never send to cloud": shown at once, put back if the core refuses. */
  const setCloudLocked = useCallback(
    async (locked: boolean) => {
      setData((d) =>
        d.detail ? { ...d, detail: { ...d.detail, cloudLocked: locked } } : d,
      );
      const r = await ipc.commands
        .setMeetingCloudLocked(id, locked)
        .catch(() => null);
      if (r?.status !== "ok")
        setData((d) =>
          d.detail
            ? { ...d, detail: { ...d.detail, cloudLocked: !locked } }
            : d,
        );
    },
    [id],
  );

  /** Sensitive mode on a stored meeting: on deletes its audio (the caller asked first). Resolves `null` when it worked, else the core's refusal word. */
  const setSensitive = useCallback(
    async (sensitive: boolean) => {
      const r = await ipc.commands
        .setMeetingSensitive(id, sensitive)
        .catch(() => null);
      if (r?.status !== "ok") return r?.error ?? "failed";
      setData((d) =>
        d.detail
          ? { ...d, detail: { ...d.detail, sensitive, audioAvailable: sensitive ? false : d.detail.audioAvailable } }
          : d,
      );
      return null;
    },
    [id],
  );

  return {
    ...data,
    chip: chipWithProgress(data.chip, percent),
    setSensitive,
    setActionDone,
    saveSegment,
    reload: load,
    setCloudLocked,
  };
}
