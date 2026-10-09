// SPDX-License-Identifier: Apache-2.0
// Who talked how much, above the transcript: a stacked bar of each speaker's
// talk time and a legend of avatar (color + initial, never color alone), name,
// share and turns. Read-only; the numbers are `talkShare` from @ghi/ui.
import { Avatar, cn, talkShare } from "@ghi/ui";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingSpeaker, SegmentView } from "../../bindings";
import { speakerOf, transcriptSpeaker } from "./notes-model";

export function TalkShareBar({
  segments,
  speakers,
}: {
  segments: readonly SegmentView[];
  speakers: readonly MeetingSpeaker[];
}) {
  const { t } = useTranslation();
  const people = useMemo(() => speakers.filter((s) => !s.notPerson), [speakers]);
  const entries = useMemo(
    () => talkShare(segments, people),
    [segments, people],
  );
  const named = entries.filter((e) => e.gid !== null).length;
  // Nobody to compare (no diarized speakers): no bar.
  if (named === 0) return null;
  const turns = entries.reduce((n, e) => n + e.turns, 0);
  const numbered = (n: number) => t("speakers.numbered", { number: n });
  return (
    <section
      aria-label={t("transcript.share.label")}
      data-testid="talk-share"
      className="flex flex-col gap-1.5 px-4 pt-2 pb-3"
    >
      <p className="text-ios-footnote m-0 text-muted">
        {t("transcript.share.summary", {
          speakers: t("transcript.share.speakers", { count: named }),
          turns: t("transcript.share.turns", { count: turns }),
        })}
      </p>
      <div
        aria-hidden="true"
        className="flex h-2 gap-px overflow-hidden rounded-full bg-sunk"
      >
        {entries.map((e) => {
          const s = e.gid ? speakerOf(speakers as MeetingSpeaker[], e.gid) : undefined;
          const slot = s?.colorSlot ?? 0;
          return (
            <span
              key={e.gid ?? "none"}
              style={{
                flexGrow: e.talkMs,
                ...(slot ? { background: `var(--s${slot})` } : {}),
              }}
              className={cn("min-w-0.5 basis-0", !slot && "bg-line2")}
            />
          );
        })}
      </div>
      <ul className="m-0 flex list-none flex-wrap gap-x-3 gap-y-1 p-0">
        {entries.map((e) => {
          const label = e.gid
            ? transcriptSpeaker(
                speakerOf(speakers as MeetingSpeaker[], e.gid),
                numbered,
                t("speakers.me"),
              )
            : null;
          return (
            <li
              key={e.gid ?? "none"}
              data-share-entry={e.gid ?? "none"}
              className="text-ios-footnote inline-flex min-h-6 items-center gap-1.5"
            >
              {label ? (
                <Avatar
                  kind={label.isMe ? "me" : "person"}
                  name={label.label}
                  initial={label.initial}
                  colorSlot={label.colorSlot}
                  size="sm"
                />
              ) : (
                <Avatar kind="unknown" size="sm" />
              )}
              <b className="font-semibold">
                {label ? label.label : t("transcript.share.unassigned")}
              </b>
              <span className="font-mono text-muted tabular-nums">{e.pct}%</span>
              <span className="text-muted">
                {t("transcript.share.turns", { count: e.turns })}
              </span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
