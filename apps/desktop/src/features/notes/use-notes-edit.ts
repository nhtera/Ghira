// SPDX-License-Identifier: Apache-2.0
// Note edits: the cache changes at once (an AI block you edit becomes yours
// on screen immediately) and rolls back, with a toast, if the core refuses.
import { useQueryClient } from "@tanstack/react-query";
import { useToast } from "@ghi/ui";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type {
  ActionItemView,
  MeetingNotes,
  NoteBlockView,
  Origin,
} from "../../bindings";
import { ipc } from "../../ipc";
import { meetingKeys } from "../../state/meeting-queries";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };

const edited = (o: Origin): Origin => (o === "ai" ? "aiEdited" : o);

export function useNotesEdit(meeting: string) {
  const client = useQueryClient();
  const { show } = useToast();
  const { t } = useTranslation();

  return useMemo(() => {
    const key = meetingKeys.notes(meeting);
    const fail = (error: string) =>
      show({
        tone: "warning",
        title: t("system.commandFailed", { message: error }),
      });
    /** Applies `change` to the cached notes, runs the command, undoes on error. */
    async function run<T>(
      change: ((n: MeetingNotes) => MeetingNotes) | null,
      call: () => Promise<Result<T>>,
    ): Promise<T | undefined> {
      const refetching = client.isFetching({ queryKey: key }) > 0;
      await client.cancelQueries({ queryKey: key });
      const before = client.getQueryData<MeetingNotes>(key);
      if (before && change) client.setQueryData(key, change(before));
      const r = await call();
      if (r.status === "ok") {
        // A cancelled refetch (e.g. after notesReady) must not leave stale notes behind.
        if (refetching) void client.invalidateQueries({ queryKey: key });
        return r.data;
      }
      if (before) client.setQueryData(key, before);
      void client.invalidateQueries({ queryKey: key });
      fail(r.error);
      return undefined;
    }
    const blocks = (
      n: MeetingNotes,
      f: (b: NoteBlockView[]) => NoteBlockView[],
    ): MeetingNotes => ({ ...n, blocks: f(n.blocks) });
    const actions = (
      n: MeetingNotes,
      f: (a: ActionItemView[]) => ActionItemView[],
    ): MeetingNotes => ({ ...n, actionItems: f(n.actionItems) });
    const patchAction =
      (item: string, p: (a: ActionItemView) => ActionItemView) =>
      (n: MeetingNotes) =>
        actions(n, (l) => l.map((a) => (a.gid === item ? p(a) : a)));

    return {
      editBlock: (gid: string, text: string) =>
        run(
          (n) =>
            blocks(n, (l) =>
              l.map((b) =>
                b.gid === gid ? { ...b, text, origin: edited(b.origin) } : b,
              ),
            ),
          () => ipc.commands.updateNoteBlock(meeting, gid, text),
        ),
      /** Decided ↔ Proposed: the decision changes kind and is now the user's. */
      setDecisionStatus: (gid: string, proposed: boolean) =>
        run(
          (n) =>
            blocks(n, (l) =>
              l.map((x) =>
                x.gid === gid
                  ? { ...x, kind: proposed ? "proposal" : "decision", origin: edited(x.origin) }
                  : x,
              ),
            ),
          () => ipc.commands.setDecisionStatus(meeting, gid, proposed),
        ),
      /** The new block, or undefined on failure. */
      addBlock: async (text: string) => {
        const b = await run(null, () =>
          ipc.commands.addNoteBlock(meeting, text),
        );
        if (b)
          client.setQueryData<MeetingNotes>(key, (n) =>
            n
              ? blocks(n, (l) =>
                  l.some((x) => x.gid === b.gid) ? l : [...l, b],
                )
              : n,
          );
        return b;
      },
      deleteBlock: (gid: string) =>
        run(
          (n) =>
            blocks(n, (l) =>
              l.filter((b) => b.gid !== gid && b.kind !== `enhanced:${gid}`),
            ),
          () => ipc.commands.deleteNoteBlock(meeting, gid),
        ),
      setDone: (item: string, done: boolean) =>
        run(
          patchAction(item, (a) => ({ ...a, done })),
          () => ipc.commands.setActionDone(meeting, item, done),
        ),
      editAction: (item: string, text: string) =>
        run(
          patchAction(item, (a) => ({ ...a, text, origin: edited(a.origin) })),
          () => ipc.commands.updateActionItem(meeting, item, text),
        ),
      setOwner: (item: string, owner: string | null) =>
        run(
          patchAction(item, (a) => ({
            ...a,
            ownerSpeakerGid: owner,
            origin: edited(a.origin),
          })),
          () => ipc.commands.setActionOwner(meeting, item, owner),
        ),
      addAction: async (text: string) => {
        const a = await run(null, () =>
          ipc.commands.addActionItem(meeting, text, null),
        );
        if (a)
          client.setQueryData<MeetingNotes>(key, (n) =>
            n
              ? actions(n, (l) =>
                  l.some((x) => x.gid === a.gid) ? l : [...l, a],
                )
              : n,
          );
        return a;
      },
      deleteAction: (item: string) =>
        run(
          (n) => actions(n, (l) => l.filter((a) => a.gid !== item)),
          () => ipc.commands.deleteActionItem(meeting, item),
        ),
    };
  }, [client, meeting, show, t]);
}

export type NotesEdit = ReturnType<typeof useNotesEdit>;
