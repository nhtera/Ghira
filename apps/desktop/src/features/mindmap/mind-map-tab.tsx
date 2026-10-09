// SPDX-License-Identifier: Apache-2.0
// The Map tab: the notes of the meeting as a mind map. Built from the notes
// that are already there (no model call): the same blocks the Notes tab shows,
// each leaf playable. "Copy as outline" is the same tree as nested Markdown.
import { notesToTree, treeToOutline, useToast, type MapNode, type NotesTreeInput } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail, NoteBlockView } from "../../bindings";
import { useMeetingNotes } from "../../state/meeting-queries";
import { usePlayer } from "../../state/player";
import { findSpeaker, speakerDisplay } from "../meeting/speaker-display";
import { layoutNotes } from "../notes/notes-model";
import { showRequests } from "../transcript/show-requests";
import { MindMap } from "./mind-map";

/** Block kinds the Notes tab places; anything else (not yours) goes to "Other". */
const KNOWN = /^(tldr|decision|question|quote|topic|note|proposal|section:.*|enhanced:.*)$/;

/** Blocks of a kind the Notes tab has no section for: kept, in the map's "Other". */
export function extraBlocks(blocks: readonly NoteBlockView[]) {
  return { other: blocks.filter((b) => b.origin !== "user" && !KNOWN.test(b.kind)) };
}

export function MindMapTab({ meeting, detail }: { meeting: string; detail: MeetingDetail }) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { show } = useToast();
  const q = useMeetingNotes(meeting);
  const notes = q.data;
  const vi = i18n.language === "vi";

  const root = useMemo(() => {
    if (!notes) return null;
    const l = layoutNotes(notes, false);
    const { other } = extraBlocks(notes.blocks);
    const input: NotesTreeInput = {
      title: detail.title,
      titles: {
        summary: t("notes.sections.summary"),
        decisions: t("notes.sections.decisions"),
        proposed: t("mindmap.sections.proposed"),
        actions: t("notes.sections.actionItems"),
        questions: t("notes.sections.openQuestions"),
        topics: t("detail.topics.title"),
        marked: t("mindmap.sections.marked"),
        other: t("mindmap.sections.other"),
      },
      tldr: l.tldr,
      sections: l.sections.map((s) => ({ id: s.section.id, title: vi ? s.section.titleVi : s.section.titleEn, blocks: s.blocks })),
      decisions: l.decisions,
      actions: l.actions,
      questions: l.questions,
      topics: l.topics,
      proposals: l.proposals,
      other,
      // The marks nothing covers, and the items that cover a mark (starred).
      marks: notes.marks.filter((k) => k.coveredBy.length === 0).map((k, i) => ({ gid: `mark-${i}`, tMs: k.tMs ?? 0, text: k.text ?? t(`notes.tags.${k.tag === "decision" || k.tag === "action" || k.tag === "question" ? k.tag : "star"}`) })),
      covered: new Set(notes.marks.flatMap((k) => k.coveredBy)),
    };
    return notesToTree(input);
  }, [notes, detail.title, vi, t]);

  const onActivate = useCallback(
    (node: MapNode, showInTranscript: boolean) => {
      const c = node.cite;
      if (c?.t0Ms == null) return;
      if (!showInTranscript && detail.audioAvailable && !c.missing) {
        usePlayer.getState().playSpan(c.t0Ms, c.t1Ms ?? c.t0Ms);
        return;
      }
      void navigate({ to: "/meetings/$id/$tab", params: { id: meeting, tab: "transcript" }, search: { t: c.t0Ms } });
      showRequests.emit(c.t0Ms);
    },
    [detail.audioAvailable, meeting, navigate],
  );

  const onCopy = useCallback(async () => {
    if (!root) return;
    try {
      const owner = (gid: string) => {
        const s = findSpeaker(detail.speakers, gid);
        return s ? speakerDisplay(s, t).name : null;
      };
      await navigator.clipboard.writeText(treeToOutline(root, owner));
      show({ tone: "success", title: t("mindmap.copied") });
    } catch (e) {
      show({ tone: "warning", title: t("system.commandFailed", { message: String(e) }) });
    }
  }, [root, show, t, detail.speakers]);

  if (q.isPending) return <div aria-busy="true" className="h-64 animate-pulse rounded-panel bg-sunk motion-reduce:animate-none" />;
  if (!notes || !root) return <p className="text-body text-muted">{t("system.commandFailed", { message: q.error?.message ?? "" })}</p>;
  if (!root.children.length) return <p className="text-body m-0 text-muted">{t("mindmap.empty")}</p>;
  return <MindMap root={root} speakers={detail.speakers} onActivate={onActivate} onCopy={() => void onCopy()} />;
}
