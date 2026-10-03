// SPDX-License-Identifier: Apache-2.0
// Search: accent-insensitive across every transcript and note on the phone
// (the core folds accents; the match is marked on the original text). Recent
// queries stay in this phone's localStorage.
import { formatClock, formatDate } from "@ghi/i18n";
import { Icon, LargeTitle, NavBar, useLargeTitleCollapse } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { SearchHitView } from "../../bindings";
import { useLocale } from "../meeting-list/format";
import { Highlighted } from "./highlight";
import { loadRecent, RECENT_CLEARED, saveRecent, withRecent } from "./recent";
import { useWindowEvent } from "../meeting-view/use-window-event";
import { useSearch } from "./use-search";

export function SearchScreen() {
  const { t } = useTranslation();
  const locale = useLocale();
  const navigate = useNavigate();
  const { collapsed, scrollRef, titleRef } = useLargeTitleCollapse();
  const [text, setText] = useState("");
  const [recent, setRecent] = useState(loadRecent);
  const search = useSearch(text);
  useWindowEvent(RECENT_CLEARED, () => setRecent([]));

  const remember = (query: string) => {
    const next = withRecent(recent, query);
    setRecent(next);
    saveRecent(next);
  };
  const open = (hit: SearchHitView) => {
    remember(text);
    void navigate({
      to: "/meetings/$id",
      params: { id: hit.meeting },
      search: {
        tab: hit.kind === "segment" ? "transcript" : "notes",
        at: hit.t0Ms ?? undefined,
      },
    });
  };

  return (
    <section data-screen="search" className="flex h-full flex-col">
      <NavBar title={t("mobile.search.title")} collapsed={collapsed} />
      <div ref={scrollRef} className="relative min-h-0 flex-1 overflow-y-auto">
        <LargeTitle ref={titleRef}>{t("mobile.search.title")}</LargeTitle>
        <form
          role="search"
          onSubmit={(e) => {
            e.preventDefault();
            remember(text);
          }}
          className="sticky top-0 z-10 bg-bg px-4 pb-2"
        >
          <div className="flex min-h-ios-target items-center gap-2 rounded-(--ios-radius-group) bg-sunk px-3">
            <Icon
              name="search"
              size={20}
              className="size-5 shrink-0 text-muted"
            />
            <input
              type="search"
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={t("mobile.search.placeholder")}
              aria-label={t("mobile.search.title")}
              autoCapitalize="none"
              autoCorrect="off"
              enterKeyHint="search"
              className="text-ios-body min-h-ios-target min-w-0 flex-1 appearance-none bg-transparent text-ink outline-none select-text placeholder:text-muted [&::-webkit-search-cancel-button]:hidden"
            />
            {text && (
              <button
                type="button"
                onClick={() => setText("")}
                aria-label={t("mobile.search.clear")}
                className="grid min-h-ios-target min-w-ios-target place-items-center text-muted"
              >
                <Icon name="close" size={18} />
              </button>
            )}
          </div>
        </form>

        {search.status === "idle" && (
          <div data-state="idle">
            {recent.length > 0 ? (
              <section aria-labelledby="recent-h" className="px-4 pt-2">
                <div className="flex items-center justify-between">
                  <h2
                    id="recent-h"
                    className="text-ios-footnote m-0 font-semibold text-muted"
                  >
                    {t("mobile.search.recent")}
                  </h2>
                  <button
                    type="button"
                    onClick={() => {
                      setRecent([]);
                      saveRecent([]);
                    }}
                    className="text-ios-subhead min-h-ios-target px-2 text-accent"
                  >
                    {t("mobile.search.clearRecent")}
                  </button>
                </div>
                <ul className="m-0 list-none p-0">
                  {recent.map((q) => (
                    <li key={q}>
                      <button
                        type="button"
                        onClick={() => setText(q)}
                        className="text-ios-body flex min-h-ios-target w-full items-center gap-3 text-start"
                      >
                        <Icon
                          name="search"
                          size={18}
                          className="size-[1.125rem] shrink-0 text-muted"
                        />
                        {q}
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : (
              <p className="text-ios-subhead m-0 px-6 py-8 text-center text-muted">
                {t("mobile.search.hint")}
              </p>
            )}
          </div>
        )}

        {search.status === "searching" && search.hits.length === 0 && (
          <p
            role="status"
            className="text-ios-subhead m-0 px-6 py-8 text-center text-muted"
          >
            {t("mobile.search.searching")}
          </p>
        )}
        {search.status === "error" && (
          <p
            role="alert"
            className="text-ios-subhead m-0 px-6 py-8 text-center"
          >
            {t("mobile.search.error")}
          </p>
        )}
        {search.status === "done" && search.hits.length === 0 && (
          <div
            data-state="no-results"
            className="flex flex-col items-center gap-2 px-6 py-10 text-center"
          >
            <Icon name="search_off" size={36} className="size-9 text-muted" />
            <h2 className="text-ios-headline m-0">
              {t("mobile.search.noResults.title", { query: search.query })}
            </h2>
            <p className="text-ios-subhead m-0 text-muted">
              {t("mobile.search.noResults.body")}
            </p>
          </div>
        )}
        {search.hits.length > 0 && (
          <div data-state="results">
            <p
              role="status"
              className="text-ios-footnote m-0 px-4 pb-1 text-muted"
            >
              {t("mobile.search.results", { count: search.hits.length })}
            </p>
            <ul className="m-0 list-none p-0">
              {search.hits.map((hit) => (
                <li
                  key={`${hit.meeting}:${hit.item}`}
                  className="border-b border-line"
                >
                  <button
                    type="button"
                    onClick={() => open(hit)}
                    className="block min-h-ios-target w-full px-4 py-3 text-start"
                  >
                    <span className="text-ios-subhead block font-semibold">
                      {hit.meetingTitle}
                    </span>
                    <span className="text-ios-caption1 block text-muted">
                      {[
                        hit.meetingStartedAt === null
                          ? null
                          : formatDate(hit.meetingStartedAt, locale),
                        hit.kind === "note"
                          ? t("mobile.search.inNotes")
                          : t("mobile.search.inTranscript"),
                        hit.kind === "segment" && hit.t0Ms !== null
                          ? formatClock(hit.t0Ms)
                          : null,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </span>
                    <span className="text-ios-body mt-1 block font-serif">
                      <Highlighted text={hit.snippet} ranges={hit.highlights} />
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </section>
  );
}
