// SPDX-License-Identifier: Apache-2.0
// The notes as an outline: a collapsible list built from `notesToTree`
// (@ghi/ui), the phone's counterpart of the computer's mind map. Read-only: a
// leaf with audio plays its first citation when tapped.
import { formatClock } from "@ghi/i18n";
import { Icon, cn, notesToTree, type MapNode, type NotesTreeInput } from "@ghi/ui";
import { useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingSpeaker } from "../../bindings";
import { speakerOf, transcriptSpeaker } from "./notes-model";

export type OutlineProps = {
  input: NotesTreeInput;
  speakers: readonly MeetingSpeaker[];
  /** Plays the audio from a moment; absent without audio here. */
  onPlayAt?: (ms: number) => void;
};

export function Outline({ input, speakers, onPlayAt }: OutlineProps) {
  const { t } = useTranslation();
  const root = useMemo(() => notesToTree(input), [input]);
  const [open, setOpen] = useState(false);
  const panel = useId();
  const leaves = root.children.reduce((n, s) => n + s.children.length, 0);
  if (root.children.length === 0) return null;

  const owner = (n: MapNode) => {
    if (n.ownerGid == null) return null;
    const s = speakerOf(speakers as MeetingSpeaker[], n.ownerGid);
    return transcriptSpeaker(s, (k) => t("speakers.numbered", { number: k }), t("speakers.me"))?.label ?? null;
  };

  const leaf = (n: MapNode) => {
    const at = n.cite?.t0Ms ?? null;
    const playable = onPlayAt != null && at != null && !n.cite?.missing;
    const who = owner(n);
    const body = (
      <>
        <span className="min-w-0 flex-1 text-start">
          {n.proposed && (
            <span
              data-testid="proposed-chip"
              className="text-ios-caption1 me-1.5 inline-flex h-5 items-center rounded-[5px] border border-dashed border-line2 px-1.5 align-middle font-semibold text-muted"
            >
              {t("mindmap.proposed")}
            </span>
          )}
          <span className={cn(n.done && "text-muted line-through")}>{n.full}</span>
          {who && <span className="text-muted">{` · ${who}`}</span>}
          {n.due && <span className="text-muted">{` · ${n.due}`}</span>}
          {n.starred && (
            <span className="text-warn">
              {" "}
              <Icon name="star" size={14} className="inline size-3.5 align-text-bottom" />
              <span className="sr-only">{t("mindmap.marked")}</span>
            </span>
          )}
        </span>
        {at != null && (
          <span className="text-ios-caption1 font-mono text-muted tabular-nums">
            {formatClock(at, { pad: true })}
          </span>
        )}
      </>
    );
    const row = "text-ios-body flex min-h-ios-target w-full items-baseline gap-2 py-1.5";
    return playable ? (
      <button
        type="button"
        onClick={() => onPlayAt(at)}
        aria-label={t("mobile.detail.playFrom", { time: formatClock(at, { pad: true }) })}
        className={row}
      >
        {body}
      </button>
    ) : (
      <div className={row}>{body}</div>
    );
  };

  return (
    <section data-testid="outline" className="px-4 pt-4">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panel}
        onClick={() => setOpen((o) => !o)}
        className="text-ios-footnote flex min-h-ios-target w-full items-center gap-1.5 font-semibold tracking-[0.07em] text-muted uppercase"
      >
        <Icon name={open ? "expand_more" : "chevron_right"} size={18} className="size-[1.125rem]" />
        {t("detail.topics.outline", { count: leaves })}
      </button>
      {open && (
        <ul id={panel} className="m-0 flex list-none flex-col gap-3 p-0 pb-2">
          {root.children.map((s) => (
            <li key={s.id}>
              <h3 className="text-ios-subhead m-0 font-semibold">{s.full}</h3>
              <ul className="m-0 flex list-none flex-col p-0 ps-3">
                {s.children.map((n) => (
                  <li key={n.id}>{leaf(n)}</li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
