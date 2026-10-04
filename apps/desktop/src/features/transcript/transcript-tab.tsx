// SPDX-License-Identifier: Apache-2.0
// Transcript tab (D6): speaker paragraphs with topic headers, karaoke while
// the audio plays (the view follows unless the user scrolled away), find with
// accent folding, inline edit and "change speaker". Virtualized: a 3-hour
// meeting is about 2,000 lines.
import { formatClock } from "@ghi/i18n";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ghi/ui";
import type { MeetingDetail, MeetingSpeaker } from "../../bindings";
import { useMeetingTranscript } from "../../state/meeting-queries";
import { usePlayer } from "../../state/player";
import { SpeakerPanel } from "../speaker-panel";
import { TopicRail, showTopicRail } from "../topic-rail";
import { FindBar } from "./find-bar";
import { GroupRow, OverlapTag, type SpeakerLabel } from "./group-row";
import type { LineRange } from "./line-text";
import { buildRows, type GroupData, findMatches, marksOf, rowIndexBySegment, segmentAt } from "./logic";
import { useLineActions } from "./use-line-actions";

const ESTIMATE_PX = 96;

/** The nearest ancestor that scrolls: the detail screen scrolls as one page, so the lines virtualize against it. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(p).overflowY)) return p;
  }
  return null;
}

/** `startAtMs`: opened from a search hit — scroll to that line once the transcript is loaded, and move the audio there once it is ready. */
export function TranscriptTab({ meeting, detail, startAtMs }: { meeting: string; detail: MeetingDetail; startAtMs?: number }) {
  const { t } = useTranslation();
  const transcript = useMeetingTranscript(meeting);
  const actions = useLineActions(meeting);
  const segments = useMemo(() => transcript.data?.segments ?? [], [transcript.data]);
  const topics = useMemo(() => transcript.data?.topics ?? [], [transcript.data]);
  const rows = useMemo(() => buildRows(segments, topics), [segments, topics]);
  const rowOf = useMemo(() => rowIndexBySegment(rows, segments.length), [rows, segments.length]);
  const marks = useMemo(() => marksOf(segments, transcript.data?.marks ?? []), [segments, transcript.data]);
  const ready = usePlayer((s) => s.meeting === meeting && !!s.src);

  const labelOf = useCallback(
    (s: MeetingSpeaker): SpeakerLabel => ({
      name: s.name ?? (s.isMe ? t("speakers.me") : t("speakers.numbered", { number: s.number })),
      initial: s.name || s.isMe ? undefined : String(s.number),
      colorSlot: s.colorSlot,
      isMe: s.isMe,
    }),
    [t],
  );
  const labels = useMemo(() => new Map(detail.speakers.map((s) => [s.gid, labelOf(s)])), [detail.speakers, labelOf]);
  const others = useMemo(() => detail.speakers.filter((s) => !s.notPerson).map((s) => ({ gid: s.gid, label: labelOf(s), numbered: !s.name && !s.isMe })), [detail.speakers, labelOf]);

  // Find.
  const [query, setQuery] = useState("");
  const [curRaw, setCur] = useState(0);
  const matches = useMemo(() => findMatches(segments, query), [segments, query]);
  const cur = Math.min(curRaw, Math.max(0, matches.length - 1));
  const ranges = useMemo(() => {
    const by = new Map<number, LineRange[]>();
    matches.forEach((m, i) => by.set(m.seg, [...(by.get(m.seg) ?? []), [m.start, m.end, i === cur] as const]));
    return by;
  }, [matches, cur]);

  // Editing and picking a speaker: one line at a time.
  const [editing, setEditing] = useState<string | null>(null);
  const [picking, setPicking] = useState<string | null>(null);
  // The speaker side panel (design 7c): who, and the line it was opened from.
  const [panel, setPanel] = useState<{ speaker: string; segment: string } | null>(null);
  const panelSpeaker = panel ? detail.speakers.find((s) => s.gid === panel.speaker) : undefined;
  const openSpeaker = useCallback((speaker: string, segment: string) => setPanel({ speaker, segment }), []);

  // Scrolling: follow the playing line until the user scrolls away.
  const list = useRef<HTMLDivElement>(null);
  const findInput = useRef<HTMLInputElement>(null);
  const [follow, setFollow] = useState(true);
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);
  const [margin, setMargin] = useState(0);
  // Where the list starts inside the scrolling page (the header above can change height).
  // Runs every render on purpose: the setters bail out when nothing moved.
  // eslint-disable-next-line react-hooks/exhaustive-deps -- measured after every render
  useLayoutEffect(() => {
    const el = list.current;
    if (!el) return;
    const p = scrollParent(el) ?? el.parentElement;
    setScrollEl(p);
    if (p) setMargin(Math.round(el.getBoundingClientRect().top - p.getBoundingClientRect().top + p.scrollTop));
  });
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollEl,
    scrollMargin: margin,
    estimateSize: () => ESTIMATE_PX,
    overscan: 10,
    initialRect: { width: 800, height: 600 },
  });
  // Wheel, touch, the scrollbar or the keys: the user takes over from the playback.
  useEffect(() => {
    if (!scrollEl) return;
    const off = () => setFollow(false);
    const onPointer = (e: PointerEvent) => e.target === scrollEl && off();
    const onKey = (e: globalThis.KeyboardEvent) => e.target === scrollEl && ["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown"].includes(e.key) && off();
    scrollEl.addEventListener("wheel", off, { passive: true });
    scrollEl.addEventListener("touchmove", off, { passive: true });
    scrollEl.addEventListener("pointerdown", onPointer);
    scrollEl.addEventListener("keydown", onKey);
    return () => {
      scrollEl.removeEventListener("wheel", off);
      scrollEl.removeEventListener("touchmove", off);
      scrollEl.removeEventListener("pointerdown", onPointer);
      scrollEl.removeEventListener("keydown", onKey);
    };
  }, [scrollEl]);
  const scrollToRow = useCallback(
    (row: number, align: "auto" | "center" | "start" = "auto") => {
      virtual.scrollToIndex(row, { align });
    },
    [virtual],
  );
  const scrollToSegment = useCallback(
    (seg: number, align: "auto" | "center" | "start" = "auto") => {
      scrollToRow(rowOf[seg] ?? 0, align);
      // The row may hold up to eight lines: bring the line itself into view once it is drawn.
      requestAnimationFrame(() => {
        const el = list.current?.querySelector(`[data-seg="${seg}"]`);
        el?.scrollIntoView?.({ block: align === "auto" ? "nearest" : "center" });
      });
    },
    [rowOf, scrollToRow],
  );

  const playing = usePlayer((s) => s.playing);
  const hasAudio = usePlayer((s) => s.src != null);
  const active = usePlayer((s) => segmentAt(segments, s.currentMs));
  useEffect(() => {
    if (playing && follow && active >= 0) scrollToSegment(active);
  }, [playing, follow, active, scrollToSegment]);
  // Any seek (a line, the waveform, a citation, play) brings the view back to the playing line.
  const seekN = usePlayer((s) => s.seekRequest?.n);
  useEffect(() => setFollow(true), [seekN]);
  // Arriving from "show in transcript" (a citation): start at the playing line.
  const arrived = useRef(false);
  useEffect(() => {
    if (arrived.current || !segments.length) return;
    arrived.current = true;
    const at = segmentAt(segments, usePlayer.getState().currentMs);
    if (at >= 0 && usePlayer.getState().currentMs > 0) scrollToSegment(at, "center");
  }, [segments, scrollToSegment]);

  const goTo = useCallback(
    (i: number) => {
      setCur(i);
      setFollow(false); // playback must not pull the view off the match
      const m = matches[i];
      if (m) scrollToSegment(m.seg, "center");
    },
    [matches, scrollToSegment],
  );
  const onQuery = (q: string) => {
    setQuery(q);
    setCur(0);
  };
  // A new query shows its first match.
  useEffect(() => {
    if (query.trim() && matches[0]) {
      setFollow(false);
      scrollToSegment(matches[0].seg, "center");
    }
    // Only when the query changes, not when an edit refetches the lines.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);
  const step = (dir: 1 | -1) => matches.length > 0 && goTo((cur + dir + matches.length) % matches.length);

  const jumpTo = useCallback(
    (tMs: number) => {
      setFollow(true);
      const at = segments.findIndex((s) => (s.t0Ms ?? 0) >= tMs);
      scrollToSegment(Math.max(0, at), "start");
      usePlayer.getState().seek(tMs, true);
    },
    [segments, scrollToSegment],
  );

  // Opened from a search hit (`?t=`): show that line as soon as the lines are
  // there (with or without audio); move the audio there once it is ready.
  const target = startAtMs;
  const scrolledTo = useRef<number | null>(null);
  const soughtTo = useRef<number | null>(null);
  useEffect(() => {
    if (target == null || !segments.length) return;
    if (scrolledTo.current !== target) {
      scrolledTo.current = target;
      setFollow(false);
      scrollToSegment(Math.max(0, segmentAt(segments, target, Infinity)), "center");
    }
    if (ready && soughtTo.current !== target) {
      soughtTo.current = target;
      usePlayer.getState().seek(target);
    }
  }, [target, segments, ready, scrollToSegment]);
  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      findInput.current?.focus();
      findInput.current?.select();
    }
  };

  const group = (g: GroupData, stacked = false) => (
    <GroupRow
      key={g.key}
      group={g}
      stacked={stacked}
      speaker={(g.speakerGid && labels.get(g.speakerGid)) || null}
      others={others}
      active={active}
      ranges={ranges}
      marks={marks}
      editing={editing}
      picking={picking}
      onEdit={setEditing}
      onPick={setPicking}
      onSave={actions.saveText}
      onSetSpeaker={actions.setSpeaker}
      onOpenSpeaker={openSpeaker}
    />
  );

  const durationMs = detail.durationMs ?? segments.at(-1)?.t1Ms ?? 0;
  const rail = showTopicRail(durationMs, topics);

  if (transcript.isPending) {
    return (
      <div role="status" aria-label={t("transcript.loading")} className="flex flex-col gap-3 p-4">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-14 animate-pulse rounded-ctl bg-sunk motion-reduce:animate-none" />
        ))}
      </div>
    );
  }
  if (transcript.isError) {
    return (
      <div role="alert" className="flex flex-col items-start gap-2 p-4 text-[13px]">
        {t("system.commandFailed", { message: String(transcript.error.message) })}
        <Button size="sm" onClick={() => void transcript.refetch()}>
          {t("common.tryAgain")}
        </Button>
      </div>
    );
  }
  if (!segments.length) return <p className="m-0 p-4 text-[13px] text-muted">{t("transcript.empty")}</p>;

  return (
    <div onKeyDown={onKeyDown} className="flex items-start gap-3">
      <div className="min-w-0 flex-1">
        <div className="sticky top-0 z-10 bg-surface pt-1 pb-2">
          <FindBar
            ref={findInput}
            query={query}
            onQuery={onQuery}
            count={matches.length}
            current={cur}
            onStep={step}
            onEscape={() => {
              setQuery("");
              list.current?.focus();
            }}
          />
        </div>
        <div ref={list} tabIndex={-1} data-testid="transcript-scroll" className="outline-none">
          <ol aria-live="off" style={{ height: virtual.getTotalSize() }} className="relative m-0 w-full list-none p-0">
            {virtual.getVirtualItems().map((v) => {
              const row = rows[v.index]!;
              return (
                <li key={row.key} data-index={v.index} ref={virtual.measureElement} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${v.start - margin}px)` }}>
                  {row.kind === "topic" ? (
                    <div className="flex items-center gap-2 px-2 pt-4 pb-1">
                      <h3 className="m-0 text-[12px] font-semibold tracking-wide text-muted uppercase">{row.title}</h3>
                      <button type="button" onClick={() => jumpTo(row.tMs)} aria-label={t("detail.playFrom", { time: formatClock(row.tMs, { pad: true }) })} className="h-6 rounded-seg px-1 text-mono text-[11px] text-muted hover:text-ink">
                        {formatClock(row.tMs, { pad: true })}
                      </button>
                      <span aria-hidden="true" className="h-px flex-1 bg-line" />
                    </div>
                  ) : row.kind === "stack" ? (
                    <div role="group" aria-label={t("transcript.overlap")} data-testid="transcript-stack" className="mx-1 my-1 rounded-l-ctl border-l-[3px] border-warn bg-warn-soft/30 pl-1">
                      <div className="flex items-center px-2 pt-1">
                        <OverlapTag />
                      </div>
                      {row.groups.map((g) => group(g, true))}
                    </div>
                  ) : (
                    group(row)
                  )}
                </li>
              );
            })}
          </ol>
        </div>
        {!follow && hasAudio && active >= 0 && (
          <div className="sticky bottom-3 z-10 flex h-0 justify-center">
            <Button
              variant="primary"
              size="sm"
              icon="arrow_upward"
              onClick={() => {
                setFollow(true);
                scrollToSegment(active, "center");
              }}
              className="-translate-y-full shadow-float"
            >
              {t("transcript.backToPlayback")}
            </Button>
          </div>
        )}
      </div>
      {rail && <TopicRail topics={topics} onJump={jumpTo} />}
      {panel && panelSpeaker && (
        <SpeakerPanel
          key={`${panel.speaker}:${panel.segment}`}
          meeting={meeting}
          mode={detail.mode}
          speaker={panelSpeaker}
          speakers={detail.speakers}
          segments={segments}
          fromSegment={panel.segment}
          onClose={() => setPanel(null)}
          onMerged={() => setPanel(null)}
        />
      )}
    </div>
  );
}
