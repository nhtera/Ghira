// SPDX-License-Identifier: Apache-2.0
// A note sentence's link to the moment it came from (brief §12). The chip
// itself plays the cited span (one click from any AI sentence to audio); hover
// or focus opens a preview with who said it, the words and a Play button. The
// preview is a plain fixed-position panel, not a popover, so it never takes
// focus from the note being edited. Chips without audio or without a matching
// moment are dashed, not playable, and say why.
import { formatClock } from "@ghi/i18n";
import { Avatar, Button, CitationChip, cn } from "@ghi/ui";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Citation, MeetingSpeaker } from "../../bindings";
import { usePlayer } from "../../state/player";
import { findSpeaker, speakerDisplay } from "../meeting/speaker-display";

const PANEL_W = 300;
const OPEN_MS = 200;
const CLOSE_MS = 150;

export type CitationLinkProps = {
  citation: Citation;
  speakers: readonly MeetingSpeaker[];
  /** False once retention removed the audio. */
  audioAvailable: boolean;
};

export const citationPlayable = (c: Citation, audioAvailable: boolean) =>
  audioAvailable && !c.missing && c.t0Ms != null;

export function CitationLink({
  citation,
  speakers,
  audioAvailable,
}: CitationLinkProps) {
  const { t } = useTranslation();
  const id = useId();
  const root = useRef<HTMLSpanElement>(null);
  const timer = useRef<number>(0);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [visited, setVisited] = useState(false);

  const playable = citationPlayable(citation, audioAvailable);
  const { t0Ms, t1Ms } = citation;
  const speaker = findSpeaker(speakers, citation.speakerGid);
  const who = speaker ? speakerDisplay(speaker, t) : null;
  const reason = citation.missing
    ? t("citation.missing")
    : !audioAvailable
      ? t("citation.noAudio")
      : citation.stale
        ? t("citation.stale")
        : null;

  const play = useCallback(() => {
    if (t0Ms == null) return;
    usePlayer.getState().playSpan(t0Ms, t1Ms ?? t0Ms);
    setVisited(true);
  }, [t0Ms, t1Ms]);

  const place = () => {
    const r = root.current?.getBoundingClientRect();
    if (!r) return;
    const left = Math.max(8, Math.min(r.left, window.innerWidth - PANEL_W - 8));
    // Below the chip, or above it near the window's bottom edge.
    setPos({
      left,
      top:
        r.bottom + 6 + 150 > window.innerHeight
          ? Math.max(8, r.top - 6 - 150)
          : r.bottom + 6,
    });
  };
  const schedule = (open: boolean) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(
      () => (open ? place() : setPos(null)),
      open ? OPEN_MS : CLOSE_MS,
    );
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  // The shared chip takes no aria props: link the open preview to its button here.
  useEffect(() => {
    const b = root.current?.querySelector("button");
    if (pos) b?.setAttribute("aria-describedby", id);
    else b?.removeAttribute("aria-describedby");
  }, [pos, id]);
  useEffect(() => {
    if (!pos) return;
    const close = () => setPos(null);
    window.addEventListener("scroll", close, true);
    return () => window.removeEventListener("scroll", close, true);
  }, [pos]);

  const broken = !playable;
  return (
    <span
      ref={root}
      data-citation={
        citation.missing ? "missing" : citation.stale ? "stale" : "ok"
      }
      className="relative inline-block whitespace-nowrap"
      onMouseEnter={() => schedule(true)}
      onMouseLeave={() => schedule(false)}
      onFocus={() => schedule(true)}
      onBlur={() => schedule(false)}
      onKeyDown={(e) => e.key === "Escape" && setPos(null)}
    >
      <CitationChip
        timeMs={citation.t0Ms ?? undefined}
        text={citation.t0Ms == null ? "?" : undefined}
        broken={broken}
        visited={visited}
        onClick={playable ? play : undefined}
        className={cn(citation.stale && !broken && "border-dotted")}
      />
      {citation.stale && !broken && (
        <span
          aria-hidden
          className="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-warn"
        />
      )}
      {pos && (
        <div
          id={id}
          role="group"
          aria-label={t("citation.preview")}
          style={{ left: pos.left, top: pos.top, width: PANEL_W }}
          className="fixed z-50 flex flex-col gap-2 rounded-panel border border-line2 bg-surface p-3 text-left whitespace-normal text-ink shadow-float"
        >
          <div className="flex items-center gap-2 text-[12.5px] text-muted">
            {who ? (
              <>
                <Avatar
                  kind={who.isMe ? "me" : "person"}
                  name={who.name}
                  initial={who.initial}
                  colorSlot={who.colorSlot}
                  size="md"
                />
                <b className="font-semibold text-ink">{who.name}</b>
              </>
            ) : (
              <Avatar kind="unknown" size="md" />
            )}
            {citation.t0Ms != null && (
              <span className="text-mono ml-auto">
                {formatClock(citation.t0Ms)}
              </span>
            )}
          </div>
          {citation.quote && (
            <p className="m-0 line-clamp-4 text-[13px] leading-snug">
              {citation.quote}
            </p>
          )}
          {reason && <p className="m-0 text-[12px] text-muted">{reason}</p>}
          {playable && (
            <Button
              size="sm"
              icon="play_arrow"
              onClick={play}
              className="self-start"
            >
              {t("detail.playFrom", { time: formatClock(citation.t0Ms ?? 0) })}
            </Button>
          )}
        </div>
      )}
    </span>
  );
}
