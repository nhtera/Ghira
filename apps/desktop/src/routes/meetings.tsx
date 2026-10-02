// SPDX-License-Identifier: Apache-2.0
// Meetings library (D3) with processing (D5): search (⌘F), filters, rows grouped
// by day, multi-select with a bulk bar, delete with Undo, and the in-place
// stepper / "Name your speakers" for meetings being processed.
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, EmptyState, Icon, shortcutLabel, useToast, usePlatform } from "@ghi/ui";
import { ipc } from "../ipc";
import { LibraryList } from "../features/library/library-list";
import { FilterBar } from "../features/library/filter-bar";
import { NO_FILTERS, applyFilters, hasFilters, peopleOf, type LibraryFilters } from "../features/library/filters";
import { SelectionBar } from "../features/library/selection-bar";
import { useMeetings } from "../features/library/use-meetings";
import { usePendingDelete } from "../features/library/use-pending-delete";
import { ExportSheet } from "../features/export/export-sheet";
import { SearchResults, type OpenHit } from "../features/search/search-results";
import { RelatedSection } from "../features/search/related-section";
import { useLibrarySearch } from "../features/search/use-library-search";
import { useRelated } from "../features/search/use-related";
import { useTemplates } from "../state/meeting-queries";
import { SHORTCUTS, matchChord } from "../shell/shortcuts";
import { NameSpeakers } from "../features/processing/name-speakers";
import { ProcessingPanel } from "../features/processing/processing-panel";
import { useProcessing } from "../features/processing/processing-store";
import { adapter } from "../features/processing/speakers-adapter";
import { Page } from "../shell/page";
import { useAppActions } from "../shell/actions";

/** Voices still unnamed in the meeting that just finished (empty until the adapter has any). */
function useUnnamed(meeting: string | undefined) {
  const q = useQuery({
    queryKey: ["unnamed-speakers", meeting],
    enabled: !!meeting,
    queryFn: () => adapter.unnamed(meeting!),
  });
  const [named, setNamed] = useState<string[]>([]);
  const left = useMemo(() => (q.data ?? []).filter((s) => !named.includes(s.gid)), [q.data, named]);
  return {
    left,
    loaded: q.isSuccess,
    markNamed: (gid: string) => setNamed((n) => [...n, gid]),
  };
}

const isTyping = (el: EventTarget | null) => el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));

export function MeetingsScreen() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const { startRecording } = useAppActions();
  const navigate = useNavigate();
  const meetings = useMeetings();
  const forget = useProcessing((s) => s.forget);
  const { show } = useToast();
  const { hidden, schedule } = usePendingDelete(forget);
  const processing = useProcessing((s) => s.meetings);
  const finished = useProcessing((s) => s.finished);
  const clearFinished = useProcessing((s) => s.clearFinished);
  const templates = useTemplates();

  const { q: initialQuery } = useSearch({ from: "/shell/meetings" });
  const [text, setText] = useState(initialQuery ?? "");
  const [filters, setFilters] = useState<LibraryFilters>(NO_FILTERS);
  const [picked, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [exporting, setExporting] = useState<string[] | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const all = useMemo(() => meetings.rows.filter((r) => !hidden.has(r.gid)), [meetings.rows, hidden]);
  const now = useMemo(() => new Date(), []);
  const rows = useMemo(() => applyFilters(all, filters, now), [all, filters, now]);
  const search = useLibrarySearch(text, filters);
  const related = useRelated(text, filters);
  const visibleIds = useMemo(() => new Set(rows.map((r) => r.gid)), [rows]);
  // Meetings that went away or are filtered out drop out of the selection.
  const selected = useMemo(() => new Set([...picked].filter((id) => visibleIds.has(id))), [picked, visibleIds]);
  // Hits for loaded meetings must pass every filter (people is client-side); meetings beyond the loaded pages are kept (the store applied the rest).
  const loadedIds = useMemo(() => new Set(meetings.rows.map((r) => r.gid)), [meetings.rows]);
  const hits = useMemo(
    () => search.hits.filter((h) => !hidden.has(h.meeting) && (!loadedIds.has(h.meeting) || visibleIds.has(h.meeting))),
    [search.hits, loadedIds, visibleIds, hidden],
  );

  // Related passages follow the same rules as keyword hits; meetings already in the hits are left out.
  const relatedShown = useMemo(
    () => related.filter((h) => !hidden.has(h.meeting.meeting) && (!loadedIds.has(h.meeting.meeting) || visibleIds.has(h.meeting.meeting))),
    [related, loadedIds, visibleIds, hidden],
  );
  const hitMeetings = useMemo(() => new Set(hits.map((h) => h.meeting)), [hits]);

  // ⌘F focuses the search; ⌘A selects every shown row; Esc clears the selection.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (matchChord(e, "Mod+F", platform)) {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      } else if (matchChord(e, "Mod+A", platform) && !isTyping(e.target) && !document.querySelector('[role="dialog"]') && !search.active) {
        e.preventDefault();
        setSelected(new Set(rows.filter((r) => r.status !== "recording" && r.status !== "processing").map((r) => r.gid)));
      } else if (e.key === "Escape" && !isTyping(e.target) && selected.size > 0 && !document.querySelector('[role="dialog"],[role="alertdialog"]')) {
        setSelected(new Set());
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [platform, rows, selected.size, search.active]);

  // The newest meeting whose notes just finished gets the naming cards.
  const naming = finished[finished.length - 1];
  const { left, loaded, markNamed } = useUnnamed(naming);
  useEffect(() => {
    if (naming && loaded && left.length === 0) clearFinished(naming);
  }, [naming, loaded, left.length, clearFinished]);

  const progress = Object.fromEntries(Object.entries(processing).map(([id, p]) => [id, p.progress ?? undefined]));
  const needsNames = useMemo(() => new Set(naming && left.length > 0 ? [naming] : []), [naming, left.length]);
  const open = useCallback((id: string) => void navigate({ to: "/meetings/$id/$tab", params: { id, tab: "notes" } }), [navigate]);
  // The detail screen reads `?t=<ms>`.
  const openHit = useCallback(
    (h: OpenHit) =>
      void navigate({
        to: "/meetings/$id/$tab",
        params: { id: h.meeting, tab: h.tab },
        search: h.tMs != null ? { t: h.tMs } : {},
      }),
    [navigate],
  );
  const titleOf = (id: string) => meetings.rows.find((r) => r.gid === id)?.title;
  const retry = async (id: string) => {
    const r = await ipc.commands.retryMeeting(id);
    if (r.status === "error")
      show({
        tone: "warning",
        title: t("system.commandFailed", { message: r.error }),
      });
    else if (r.data === 0) show({ title: t("library.nothingToRetry") });
    void meetings.refetch();
  };
  const copyNotes = async (id: string) => {
    const r = await ipc.commands.meetingAsText(id, true, {
      notes: true,
      transcript: false,
      vietnamese: i18n.language.startsWith("vi"),
    });
    if (r.status === "error")
      return show({
        tone: "warning",
        title: t("system.commandFailed", { message: r.error }),
      });
    try {
      await navigator.clipboard.writeText(r.data);
      show({ tone: "success", title: t("library.notesCopied") });
    } catch (e) {
      show({
        tone: "warning",
        title: t("system.commandFailed", { message: String(e) }),
      });
    }
  };
  const processingIds = Object.keys(processing);
  const filtering = hasFilters(filters);
  const clearFilters = () => setFilters(NO_FILTERS);

  const top = (
    <>
      {processingIds.map((id) => (
        <ProcessingPanel
          key={id}
          title={titleOf(id)}
          processing={processing[id]!}
          waitingForModels={meetings.rows.find((r) => r.gid === id)?.job?.waitingForModels}
        />
      ))}
      {naming && left.length > 0 && <NameSpeakers meeting={naming} speakers={left} onDone={markNamed} onSkipAll={() => clearFinished(naming)} />}
    </>
  );
  const empty = meetings.isSuccess && meetings.rows.length === 0 && processingIds.length === 0;

  return (
    <Page
      title={t("nav.meetings")}
      subtitle={meetings.isSuccess ? t("library.subtitle", { context: platform, count: meetings.rows.length }) : undefined}
      actions={
        <>
          <Button icon="upload_file" onClick={() => void navigate({ to: "/import" })}>
            {t("library.empty.importFile")}
          </Button>
          <Button variant="primary" icon="mic" onClick={() => void startRecording()}>
            {t("library.newRecording")}
          </Button>
        </>
      }
    >
      {/* The page body scrolls by itself; the library fills it and scrolls inside, under the search and filters. */}
      <div className="flex h-full min-h-0 flex-col">
        {empty ? (
          <EmptyState kind="library" className="mt-10" onPrimary={() => void startRecording("call")} onSecondary={() => void navigate({ to: "/import" })} />
        ) : (
          <>
            <div className="flex flex-none flex-col gap-2.5 pb-3">
              <label className="flex h-10 items-center gap-2 rounded-panel border border-line2 bg-surface px-3 focus-within:border-accent">
                <Icon name="search" size={19} className="text-faint" />
                <input
                  ref={searchRef}
                  type="search"
                  value={text}
                  onChange={(e) => {
                    setText(e.target.value);
                    // The `?q=` from Ask was only the starting text: drop it so Back doesn't bring it back.
                    if (initialQuery) void navigate({ to: "/meetings", search: {}, replace: true });
                  }}
                  onKeyDown={(e) => e.key === "Escape" && text && (e.stopPropagation(), setText(""))}
                  placeholder={t("library.searchPlaceholder")}
                  aria-label={t("library.searchPlaceholder")}
                  className="min-w-0 flex-1 border-0 bg-transparent text-[14px] text-ink outline-none"
                />
                <kbd className="text-mono hidden text-faint sm:inline">{shortcutLabel(SHORTCUTS.find, platform)}</kbd>
              </label>
              <FilterBar
                filters={filters}
                onChange={setFilters}
                people={peopleOf(all)}
                templates={(templates.data ?? []).map((x) => ({
                  value: x.id,
                  label: x.name,
                }))}
                onClear={clearFilters}
              />
              {selected.size > 0 && (
                <SelectionBar
                  count={selected.size}
                  onExport={() => setExporting([...selected])}
                  onDelete={() => (schedule([...selected]), setSelected(new Set()))}
                  onClear={() => setSelected(new Set())}
                />
              )}
            </div>
            {search.active ? (
              <div className="min-h-0 flex-1 overflow-auto" aria-busy={search.loading}>
                {top}
                {search.error ? (
                  <p className="text-body text-muted">{t("system.commandFailed", { message: search.error })}</p>
                ) : hits.length > 0 ? (
                  <SearchResults hits={hits} hasMore={!!search.hasMore} loadingMore={search.loadingMore} onMore={search.more} onOpen={openHit} />
                ) : (
                  !search.loading && (
                    <NoMatches
                      icon="search_off"
                      title={t("library.emptySearch.title", {
                        query: search.query,
                      })}
                      body={t("library.emptySearch.body")}
                      filtering={filtering}
                      onClear={clearFilters}
                    />
                  )
                )}
                <RelatedSection hits={relatedShown} exclude={hitMeetings} onOpen={openHit} />
              </div>
            ) : rows.length === 0 && filtering ? (
              <div className="min-h-0 flex-1 overflow-auto">
                {top}
                <NoMatches icon="search_off" title={t("library.noFilterMatches")} filtering onClear={clearFilters} />
              </div>
            ) : (
              <LibraryList
                rows={rows}
                header={top}
                progress={progress}
                needsNames={needsNames}
                selected={selected}
                onSelectionChange={setSelected}
                onOpen={open}
                onExport={(id) => setExporting([id])}
                onCopyNotes={(id) => void copyNotes(id)}
                onDelete={(id) => schedule([id])}
                onRetry={(id) => void retry(id)}
              />
            )}
          </>
        )}
      </div>
      <ExportSheet open={exporting != null} onOpenChange={(o) => !o && setExporting(null)} meetings={exporting ?? []} />
    </Page>
  );
}

function NoMatches({ icon, title, body, filtering, onClear }: { icon: "search_off"; title: string; body?: string; filtering: boolean; onClear: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center gap-2 py-12 text-center text-muted">
      <Icon name={icon} size={32} className="text-faint" />
      <b className="text-body font-semibold text-ink">{title}</b>
      {body && <span className="text-small">{body}</span>}
      {filtering && <Button onClick={onClear}>{t("library.clearFilters")}</Button>}
    </div>
  );
}
