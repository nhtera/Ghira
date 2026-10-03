// SPDX-License-Identifier: Apache-2.0
// D3 search results: the library's rows for the meetings that matched, in the
// store's relevance order, the title and the best snippet with <mark>ed matches. A meeting with
// more hits lists them under its row; segment hits open the transcript at their
// time, note hits open Notes. Text nodes only (RT-6).
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { formatClock, type Locale } from "@ghi/i18n";
import { Button, Icon } from "@ghi/ui";
import type { MeetingRow, SearchHitView } from "../../bindings";
import { MeetingRowView } from "../library/meeting-row";
import { rowStatus } from "../library/meeting-status";
import { groupHits, type HitGroup } from "./group-hits";
import { queryRanges, splitHighlights } from "./highlight";

export type OpenHit = {
  meeting: string;
  tab: "notes" | "transcript";
  tMs: number | null;
};

function Marked({ text, ranges }: { text: string; ranges: readonly (readonly [number, number])[] }) {
  return (
    <>
      {splitHighlights(text, ranges).map((p, i) =>
        p.mark ? (
          <mark key={i} className="rounded-[3px] bg-warn-soft text-inherit">
            {p.text}
          </mark>
        ) : (
          p.text
        ),
      )}
    </>
  );
}

export function Snippet({ hit }: { hit: Pick<SearchHitView, "snippet" | "highlights"> }) {
  return <Marked text={hit.snippet} ranges={hit.highlights} />;
}

const openOf = (h: SearchHitView): OpenHit => ({
  meeting: h.meeting,
  tab: h.kind === "segment" ? "transcript" : "notes",
  tMs: h.kind === "segment" ? h.t0Ms : null,
});

/** A meeting beyond the loaded pages: the hit knows its title and date; the row shows no status. */
const rowOfGroup = (g: HitGroup): MeetingRow => ({
  gid: g.meeting,
  title: g.title,
  startedAt: g.startedAt,
  durationMs: null,
  source: "live",
  mode: "call",
  status: "ready",
  transcriptVersion: null,
  cloudUsed: false,
  consentConfirmed: false,
  template: null,
  people: [],
  job: null,
  folder: null,
  tags: [],
  sourceApp: null,
  summary: null,
});

function MoreHits({ hits, onOpen }: { hits: SearchHitView[]; onOpen: (h: OpenHit) => void }) {
  const { t } = useTranslation();
  return (
    <ul className="m-0 mb-1 flex list-none flex-col p-0 pl-[78px]">
      {hits.map((h) => (
        <li key={`${h.kind}:${h.item}`}>
          <button type="button" onClick={() => onOpen(openOf(h))} className="flex min-h-8 w-full items-baseline gap-2.5 rounded-seg px-2 py-1 text-left hover:bg-surface2">
            {h.kind === "segment" ? (
              h.t0Ms != null && <span className="text-mono flex-none text-[12px] text-faint">{formatClock(h.t0Ms)}</span>
            ) : (
              <span className="text-small flex flex-none items-center gap-1 self-center text-muted">
                <Icon name="description" size={15} />
                {t("library.hitNote")}
              </span>
            )}
            <span className="min-w-0 flex-1 font-serif text-[14.5px] text-muted [overflow-wrap:anywhere]">
              <Snippet hit={h} />
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

export function SearchResults({
  hits,
  query,
  rows = [],
  locale = "en",
  hasMore,
  loadingMore,
  onMore,
  onOpen,
}: {
  hits: SearchHitView[];
  /** What was searched: the title is highlighted with it. */
  query: string;
  /** The library rows loaded so far (status, people and duration of the matched meetings). */
  rows?: readonly MeetingRow[];
  locale?: Locale;
  hasMore: boolean;
  loadingMore: boolean;
  onMore: () => void;
  onOpen: (h: OpenHit) => void;
}) {
  const { t } = useTranslation();
  // The store's relevance order, meeting by meeting; "More results" appends below.
  const matches = useMemo(() => {
    const known = new Map(rows.map((r) => [r.gid, r]));
    return groupHits(hits).map((g) => ({ group: g, row: known.get(g.meeting) ?? rowOfGroup(g), loaded: known.has(g.meeting) }));
  }, [hits, rows]);
  return (
    <div>
      <p className="text-small m-0 mb-2.5 text-muted" aria-live="polite">
        {t("library.results", { count: matches.length })}
      </p>
      {matches.map(({ group: g, row, loaded }) => {
        const [first, ...rest] = g.hits;
        const title = row.title || t("live.titlePlaceholder");
        return (
          <div key={row.gid} role="group" aria-label={title}>
            <MeetingRowView
              row={row}
              status={loaded ? rowStatus(row, undefined, false) : null}
              title={<Marked text={title} ranges={queryRanges(title, query)} />}
              line={first && <Snippet hit={first} />}
              locale={locale}
              onOpen={() => onOpen(first ? openOf(first) : { meeting: row.gid, tab: "notes", tMs: null })}
            />
            {rest.length > 0 && <MoreHits hits={rest} onOpen={onOpen} />}
          </div>
        );
      })}
      {hasMore && (
        <div className="flex justify-center py-2">
          <Button disabled={loadingMore} onClick={onMore}>
            {t("library.moreResults")}
          </Button>
        </div>
      )}
    </div>
  );
}
