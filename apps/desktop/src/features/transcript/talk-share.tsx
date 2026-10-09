// SPDX-License-Identifier: Apache-2.0
// Who talked how much (design: Meeting Studio's share bar): a stacked bar of
// each speaker's talk time above the transcript, and a legend of avatar (color
// + initial, never color alone), name, share and turns. A named legend entry
// opens that speaker's panel.
import { Avatar, cn } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { SpeakerLabel } from "./group-row";
import type { ShareEntry } from "./logic";

export function TalkShare({
  entries,
  labels,
  speakerCount,
  onOpenSpeaker,
}: {
  entries: readonly ShareEntry[];
  labels: ReadonlyMap<string, SpeakerLabel>;
  /** People among the entries (not video or music, not unassigned). */
  speakerCount: number;
  onOpenSpeaker: (speaker: string) => void;
}) {
  const { t } = useTranslation();
  // Nobody to compare (no diarized speakers): no bar.
  if (!entries.length || speakerCount === 0) return null;
  const turns = entries.reduce((n, e) => n + e.turns, 0);
  return (
    <section aria-label={t("transcript.share.label")} data-testid="talk-share" className="flex flex-col gap-1.5 px-1 pb-2">
      <p className="m-0 text-[12px] text-muted">
        {t("transcript.share.summary", {
          speakers: t("transcript.share.speakers", { count: speakerCount }),
          turns: t("transcript.share.turns", { count: turns }),
        })}
      </p>
      <div aria-hidden="true" className="flex h-2 gap-px overflow-hidden rounded-full bg-sunk">
        {entries.map((e) => {
          const label = e.gid ? labels.get(e.gid) : undefined;
          const slot = label?.colorSlot ?? 0;
          return <span key={e.gid ?? "none"} data-share={e.gid ?? "none"} style={{ flexGrow: e.talkMs, ...(slot ? { background: `var(--s${slot})` } : {}) }} className={cn("min-w-0.5 basis-0", !slot && "bg-line2")} />;
        })}
      </div>
      <ul className="m-0 flex list-none flex-wrap gap-x-3 gap-y-1 p-0">
        {entries.map((e) => {
          const label = e.gid ? labels.get(e.gid) : undefined;
          const name = label?.name ?? t("transcript.share.unassigned");
          const body = (
            <>
              {label ? <Avatar kind={label.isMe ? "me" : "person"} name={label.name} initial={label.initial} colorSlot={label.colorSlot} size="sm" /> : <Avatar kind="unknown" size="sm" />}
              <span className="text-[12.5px] font-semibold text-ink">{name}</span>
              <span className="text-mono text-[12px] text-muted">{e.pct}%</span>
              <span className="text-[12px] text-muted">{t("transcript.share.turns", { count: e.turns })}</span>
            </>
          );
          return (
            <li key={e.gid ?? "none"} data-share-entry={e.gid ?? "none"}>
              {e.gid && label ? (
                <button type="button" aria-label={t("transcript.share.entry", { name, pct: e.pct, turns: t("transcript.share.turns", { count: e.turns }) })} onClick={() => onOpenSpeaker(e.gid!)} className="inline-flex min-h-6 items-center gap-1.5 rounded-seg px-1 hover:bg-sunk">
                  {body}
                </button>
              ) : (
                <span className="inline-flex min-h-6 items-center gap-1.5 rounded-seg px-1">{body}</span>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
