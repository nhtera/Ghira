// SPDX-License-Identifier: Apache-2.0
// A note sentence's link to the moment it came from (brief §12). The chip
// itself plays the cited span (one click from any AI sentence to audio); hover
// or focus opens a preview with who said it, the words and a Play button. The
// preview is a plain fixed-position panel, not a popover, so it never takes
// focus from the note being edited. Chips without audio or without a matching
// moment are dashed, not playable, and say why. An item with several sources
// shows the first chip plus a "+n" chip; the preview steps through all of them
// (‹ k/n ›, also with ←/→) and can show the moment in the transcript.
import { formatClock } from "@ghi/i18n";
import { Avatar, Button, CitationChip, cn } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { Citation, MeetingSpeaker } from "../../bindings";
import { usePlayer } from "../../state/player";
import { showRequests } from "../transcript/show-requests";
import { findSpeaker, speakerDisplay } from "../meeting/speaker-display";

const PANEL_W = 300;
const OPEN_MS = 200;
const CLOSE_MS = 150;

export type CitationGroupProps = {
  citations: readonly Citation[];
  speakers: readonly MeetingSpeaker[];
  /** False once retention removed the audio. */
  audioAvailable: boolean;
  /** The meeting, for "Show in transcript" (the button is left out without it). */
  meeting?: string;
};

export type CitationLinkProps = Omit<CitationGroupProps, "citations"> & { citation: Citation };

export const citationPlayable = (c: Citation, audioAvailable: boolean) =>
  audioAvailable && !c.missing && c.t0Ms != null;

/** The transcript tab at the cited moment (its line is centered and pulses). */
function ShowInTranscript({ meeting, tMs }: { meeting: string; tMs: number }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <Button
      size="sm"
      variant="ghost"
      icon="description"
      onClick={() => {
        void navigate({ to: "/meetings/$id/$tab", params: { id: meeting, tab: "transcript" }, search: { t: tMs } });
        // Already on that moment: the route does not change, so tell a mounted transcript directly.
        showRequests.emit(tMs);
      }}
    >
      {t("citation.showInTranscript")}
    </Button>
  );
}

export function CitationLink({ citation, ...rest }: CitationLinkProps) {
  return <CitationGroup citations={[citation]} {...rest} />;
}

export function CitationGroup({
  citations,
  speakers,
  audioAvailable,
  meeting,
}: CitationGroupProps) {
  const { t } = useTranslation();
  const n = citations.length;
  const [at, setAt] = useState(0);
  const k = Math.min(at, n - 1);
  const citation = citations[k]!;
  const step = (dir: 1 | -1) => setAt((k + dir + n) % n);
  const first = citations[0]!;
  const id = useId();
  const root = useRef<HTMLSpanElement>(null);
  const timer = useRef<number>(0);
  const [pos, setPos] = useState<{ left: number; top: number; chipTop: number; chipBottom: number } | null>(null);
  const [visited, setVisited] = useState(false);

  const playable = citationPlayable(citation, audioAvailable);
  const firstPlayable = citationPlayable(first, audioAvailable);
  const speaker = findSpeaker(speakers, citation.speakerGid);
  const who = speaker ? speakerDisplay(speaker, t) : null;
  const reason = citation.missing
    ? t("citation.missing")
    : !audioAvailable
      ? t("citation.noAudio")
      : citation.stale
        ? t("citation.stale")
        : null;

  const play = useCallback((c: Citation) => {
    if (c.t0Ms == null) return;
    usePlayer.getState().playSpan(c.t0Ms, c.t1Ms ?? c.t0Ms);
    setVisited(true);
  }, []);

  const place = () => {
    const r = root.current?.getBoundingClientRect();
    if (!r) return;
    const left = Math.max(8, Math.min(r.left, window.innerWidth - PANEL_W - 8));
    // Below the chip, or above it near the window's bottom edge.
    // `top` is a first guess below the chip; the layout effect below flips it above once the real height is known.
    setPos({ left, top: r.bottom + 6, chipTop: r.top, chipBottom: r.bottom });
  };
  // Closing forgets the stepped-to source: the next preview starts at the first.
  const hide = () => {
    setPos(null);
    setAt(0);
  };
  const schedule = (open: boolean) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(
      () => (open ? place() : hide()),
      open ? OPEN_MS : CLOSE_MS,
    );
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const panel = useRef<HTMLDivElement>(null);
  // Below the chip, or above it when the (measured) preview would pass the window's bottom edge.
  useLayoutEffect(() => {
    const el = panel.current;
    if (!el || !pos) return;
    const h = el.offsetHeight;
    el.style.top = `${pos.chipBottom + 6 + h > window.innerHeight ? Math.max(8, pos.chipTop - 6 - h) : pos.chipBottom + 6}px`;
  });

  // The shared chip takes no aria props: link the open preview to its button here.
  useEffect(() => {
    const b = root.current?.querySelector("button");
    if (pos) b?.setAttribute("aria-describedby", id);
    else b?.removeAttribute("aria-describedby");
  }, [pos, id]);
  useEffect(() => {
    if (!pos) return;
    const close = hide;
    window.addEventListener("scroll", close, true);
    return () => window.removeEventListener("scroll", close, true);
  }, [pos]);

  const broken = !firstPlayable;
  const onKeyDown = (e: KeyboardEvent<HTMLSpanElement>) => {
    if (e.key === "Escape") return hide();
    // ←/→ step through the sources while the preview is open.
    if (pos && n > 1 && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      step(e.key === "ArrowRight" ? 1 : -1);
    }
  };
  return (
    <span
      ref={root}
      data-citation={first.missing ? "missing" : first.stale ? "stale" : "ok"}
      className="relative inline-block whitespace-nowrap"
      onMouseEnter={() => schedule(true)}
      onMouseLeave={() => schedule(false)}
      onFocus={() => schedule(true)}
      onBlur={() => schedule(false)}
      onKeyDown={onKeyDown}
    >
      <CitationChip
        timeMs={first.t0Ms ?? undefined}
        text={first.t0Ms == null ? "?" : undefined}
        broken={broken}
        visited={visited}
        onClick={firstPlayable ? () => play(first) : undefined}
        className={cn(first.stale && !broken && "border-dotted")}
      />
      {n > 1 && (
        <button
          type="button"
          data-citation-more
          aria-label={t("citation.more", { count: n - 1 })}
          onClick={() => {
            setAt(1);
            place();
          }}
          className="text-mono ml-0.5 inline-flex h-6 min-w-6 items-center justify-center rounded-seg border border-line2 bg-surface2 px-1 align-middle text-[12px] text-muted transition-colors duration-(--motion-fast) ease-out hover:border-accent hover:text-accent"
        >
          {`+${n - 1}`}
        </button>
      )}
      {first.stale && !broken && (
        <span
          aria-hidden
          className="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-warn"
        />
      )}
      {pos && (
        <div
          ref={panel}
          id={id}
          role="group"
          aria-label={t("citation.preview")}
          style={{ left: pos.left, width: PANEL_W }}
          className="fixed z-50 flex flex-col gap-2 rounded-panel border border-line2 bg-surface p-3 text-left whitespace-normal text-ink shadow-float"
        >
          <div className="flex items-center gap-2 text-[12.5px] text-muted">
            {n > 1 && (
              <div className="mr-1 flex items-center">
                <Button size="sm" variant="ghost" icon="chevron_left" aria-label={t("citation.prev")} onClick={() => step(-1)} />
                <span role="status" className="text-mono text-[12px] text-ink">
                  <span aria-hidden="true">{`${k + 1}/${n}`}</span>
                  <span className="sr-only">{t("citation.step", { k: k + 1, total: n })}</span>
                </span>
                <Button size="sm" variant="ghost" icon="chevron_right" aria-label={t("citation.next")} onClick={() => step(1)} />
              </div>
            )}
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
                {formatClock(citation.t0Ms, { pad: true })}
              </span>
            )}
          </div>
          {citation.quote && (
            <p className="m-0 line-clamp-4 text-[13px] leading-snug">
              {citation.quote}
            </p>
          )}
          {reason && <p className="m-0 text-[12px] text-muted">{reason}</p>}
          {(playable || (meeting && citation.t0Ms != null && !citation.missing)) && (
            <div className="flex flex-wrap items-center gap-1">
              {playable && (
                <Button size="sm" icon="play_arrow" onClick={() => play(citation)}>
                  {t("detail.playFrom", { time: formatClock(citation.t0Ms ?? 0, { pad: true }) })}
                </Button>
              )}
              {meeting && citation.t0Ms != null && !citation.missing && <ShowInTranscript meeting={meeting} tMs={citation.t0Ms} />}
            </div>
          )}
        </div>
      )}
    </span>
  );
}
