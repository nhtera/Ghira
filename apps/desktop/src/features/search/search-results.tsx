// SPDX-License-Identifier: Apache-2.0
// D3 search results: grouped by meeting, each hit a snippet with <mark>ed
// matches. Segment hits carry speaker + time and open the transcript there;
// note hits open Notes. Exact accented matches come first (the store ranks).
import { useTranslation } from "react-i18next";
import { formatClock, formatDate, type Locale } from "@ghi/i18n";
import { Avatar, Button, Icon } from "@ghi/ui";
import type { MeetingSpeaker, SearchHitView } from "../../bindings";
import { useMeetingDetail } from "../../state/meeting-queries";
import { groupHits, type HitGroup } from "./group-hits";
import { splitHighlights } from "./highlight";

export type OpenHit = {
  meeting: string;
  tab: "notes" | "transcript";
  tMs: number | null;
};

export function Snippet({ hit }: { hit: Pick<SearchHitView, "snippet" | "highlights"> }) {
  return (
    <>
      {splitHighlights(hit.snippet, hit.highlights).map((p, i) =>
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

function SpeakerTag({ speaker }: { speaker: MeetingSpeaker | undefined }) {
  const { t } = useTranslation();
  if (!speaker) return null;
  const name = speaker.isMe ? t("speakers.me") : (speaker.name ?? t("speakers.numbered", { number: speaker.number }));
  return (
    <span className="flex flex-none items-center gap-1.5">
      <Avatar
        kind={speaker.isMe ? "me" : "person"}
        name={name}
        initial={speaker.name ? undefined : String(speaker.number)}
        colorSlot={speaker.colorSlot}
        size="sm"
      />
      <span className="text-small font-semibold text-ink">{name}</span>
    </span>
  );
}

// Names for the speaker tags: one cached detail query per meeting that has segment hits.
function Group({ group, onOpen }: { group: HitGroup; onOpen: (h: OpenHit) => void }) {
  return group.hits.some((h) => h.kind === "segment") ? <GroupWithSpeakers group={group} onOpen={onOpen} /> : <GroupBody group={group} onOpen={onOpen} />;
}

function GroupWithSpeakers({ group, onOpen }: { group: HitGroup; onOpen: (h: OpenHit) => void }) {
  const detail = useMeetingDetail(group.meeting);
  return <GroupBody group={group} onOpen={onOpen} speakers={detail.data?.speakers} />;
}

function GroupBody({ group, onOpen, speakers }: { group: HitGroup; onOpen: (h: OpenHit) => void; speakers?: MeetingSpeaker[] }) {
  const { t, i18n } = useTranslation();
  const locale = (i18n.language === "vi" ? "vi" : "en") as Locale;
  return (
    <section aria-label={group.title} className="mb-3 rounded-row border border-line bg-surface">
      <button
        type="button"
        onClick={() => onOpen({ meeting: group.meeting, tab: "notes", tMs: null })}
        className="flex w-full items-center gap-2 rounded-row px-3 py-2 text-left hover:bg-surface2"
      >
        <Icon name="graphic_eq" size={18} className="flex-none text-muted" />
        <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold">{group.title}</span>
        {group.startedAt != null && <span className="text-small flex-none text-muted">{formatDate(group.startedAt, locale)}</span>}
      </button>
      <ul className="m-0 flex list-none flex-col p-0 pb-1">
        {group.hits.map((h) => (
          <li key={`${h.kind}:${h.item}`}>
            <button
              type="button"
              onClick={() =>
                onOpen({
                  meeting: h.meeting,
                  tab: h.kind === "segment" ? "transcript" : "notes",
                  tMs: h.kind === "segment" ? h.t0Ms : null,
                })
              }
              className="flex min-h-9 w-full items-baseline gap-2.5 px-3 py-1.5 text-left hover:bg-surface2"
            >
              {h.kind === "segment" ? (
                <span className="flex flex-none items-center gap-2 self-center">
                  <SpeakerTag speaker={speakers?.find((s) => s.gid === h.speakerGid)} />
                  {h.t0Ms != null && <span className="text-mono text-muted">{formatClock(h.t0Ms)}</span>}
                </span>
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
    </section>
  );
}

export function SearchResults({
  hits,
  hasMore,
  loadingMore,
  onMore,
  onOpen,
}: {
  hits: SearchHitView[];
  hasMore: boolean;
  loadingMore: boolean;
  onMore: () => void;
  onOpen: (h: OpenHit) => void;
}) {
  const { t } = useTranslation();
  const groups = groupHits(hits);
  return (
    <div>
      <p className="text-small m-0 mb-2.5 text-muted" aria-live="polite">
        {t("library.results", { count: hits.length })}
      </p>
      {groups.map((g) => (
        <Group key={g.meeting} group={g} onOpen={onOpen} />
      ))}
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
