// SPDX-License-Identifier: Apache-2.0
// M3: the meetings list. Large title, the design's search field (it opens the
// Search tab's screen with the keyboard up: one search, two doors), rows grouped
// by day under uppercase headers (virtualized, so 1,000+ meetings scroll), pull
// to refresh, and the empty / loading / error states.
import {
  Banner,
  Button,
  Icon,
  LargeTitle,
  NavBar,
  useLargeTitleCollapse,
} from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type TouchEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { useDayLabel } from "./format";
import { pullState, withDayHeaders } from "./group";
import { MeetingRowView } from "./meeting-row";
import { useMeetingList } from "./use-meeting-list";

const HEADER_H = 40;
const ROW_H = 104;
/** Fetch the next page when this close to the end. */
const AHEAD = 15;

export function MeetingList() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const list = useMeetingList();
  const dayLabel = useDayLabel();
  const { collapsed, scrollRef, titleRef } = useLargeTitleCollapse();
  const listRef = useRef<HTMLDivElement>(null);
  const [margin, setMargin] = useState(0);
  const items = useMemo(
    () => withDayHeaders(list.rows, Date.now()),
    [list.rows],
  );
  const empty = list.status === "ready" && list.rows.length === 0;

  // Where the list starts inside the scroller (below the large title).
  useLayoutEffect(() => {
    if (listRef.current) setMargin(listRef.current.offsetTop);
  }, [list.status, empty]);

  // eslint-disable-next-line react-hooks/incompatible-library -- the virtualizer is only used in this component
  const virtual = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (items[i].type === "header" ? HEADER_H : ROW_H),
    getItemKey: (i) => items[i].id,
    scrollMargin: margin,
    overscan: 8,
  });
  const vItems = virtual.getVirtualItems();
  const lastIndex = vItems.length ? vItems[vItems.length - 1].index : -1;
  const { status, done, loadMore } = list;
  useEffect(() => {
    if (status === "ready" && !done && lastIndex >= items.length - AHEAD)
      void loadMore();
  }, [status, done, lastIndex, items.length, loadMore]);

  // Pull to refresh: a downward drag from the top of the list.
  const [pull, setPull] = useState(0);
  const drag = useRef<{ y: number; ready: boolean } | null>(null);
  const touchStart = (e: TouchEvent) => {
    if ((scrollRef.current?.scrollTop ?? 1) <= 0)
      drag.current = { y: e.touches[0].clientY, ready: false };
  };
  const touchMove = (e: TouchEvent) => {
    if (!drag.current) return;
    const s = pullState(e.touches[0].clientY - drag.current.y);
    drag.current.ready = s.ready;
    setPull(s.pull);
  };
  const touchEnd = () => {
    if (drag.current?.ready) void list.refresh();
    drag.current = null;
    setPull(0);
  };

  const indicator = list.refreshing || pull > 0;
  return (
    <section data-screen="meetings" className="flex h-full flex-col">
      <NavBar title={t("mobile.meetings.title")} collapsed={collapsed} />
      <div
        ref={scrollRef}
        onTouchStart={touchStart}
        onTouchMove={touchMove}
        onTouchEnd={touchEnd}
        onTouchCancel={touchEnd}
        className="relative min-h-0 flex-1 overflow-y-auto"
      >
        <LargeTitle ref={titleRef} className="tracking-[-0.01em]!">
          {t("mobile.meetings.title")}
        </LargeTitle>
        {!empty && list.status !== "loading" && (
          <button
            type="button"
            onClick={() =>
              void navigate({ to: "/search", search: { focus: true } })
            }
            className="text-ios-body mx-4 mb-2 flex min-h-ios-target w-[calc(100%-2rem)] items-center gap-2 rounded-(--ios-radius-group) bg-sunk px-3 text-start text-muted"
          >
            <Icon name="search" size={20} className="size-5 shrink-0" />
            {t("mobile.meetings.searchPlaceholder")}
          </button>
        )}
        {indicator && (
          <p
            role="status"
            style={{ height: list.refreshing ? 40 : pull }}
            className="text-ios-footnote m-0 flex items-center justify-center gap-2 overflow-hidden text-muted"
          >
            <Icon
              name="progress_activity"
              size={16}
              className={
                list.refreshing ? "animate-spin motion-reduce:animate-none" : ""
              }
            />
            {list.refreshing
              ? t("mobile.meetings.refreshing")
              : t("mobile.meetings.pull")}
          </p>
        )}
        {list.deleteFailed && (
          <div className="px-4 pb-2">
            <Banner
              variant="warning"
              title={t("mobile.meetings.deleteFailed")}
              onDismiss={list.dismissDeleteFailed}
              dismissLabel={t("mobile.banner.dismiss")}
            />
          </div>
        )}
        {list.status === "loading" && (
          <p
            role="status"
            className="text-ios-subhead m-0 px-4 py-6 text-center text-muted"
          >
            {t("mobile.meetings.loading")}
          </p>
        )}
        {list.status === "error" && (
          <div className="flex flex-col items-center gap-3 px-4 py-8 text-center">
            <p role="alert" className="text-ios-subhead m-0">
              {t("mobile.meetings.error")}
            </p>
            <Button
              variant="primary"
              className="min-h-ios-target"
              onClick={() => void list.refresh()}
            >
              {t("mobile.meetings.retry")}
            </Button>
          </div>
        )}
        {empty && (
          <div
            data-state="empty"
            className="flex flex-col items-center gap-3 px-6 py-12 text-center"
          >
            <Icon name="graphic_eq" size={44} className="size-11 text-accent" />
            <h2 className="text-ios-title3 m-0">
              {t("mobile.meetings.empty.title")}
            </h2>
            <p className="text-ios-subhead m-0 max-w-sm text-muted">
              {t("mobile.meetings.empty.body")}
            </p>
            <Button
              variant="primary"
              icon="mic"
              className="min-h-ios-target px-5"
              onClick={() => void navigate({ to: "/record" })}
            >
              {t("mobile.meetings.empty.record")}
            </Button>
          </div>
        )}
        {items.length > 0 && (
          <div
            ref={listRef}
            style={{ height: virtual.getTotalSize(), position: "relative" }}
          >
            {vItems.map((v) => {
              const item = items[v.index];
              return (
                <div
                  key={v.key}
                  data-index={v.index}
                  ref={virtual.measureElement}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${v.start - margin}px)`,
                  }}
                >
                  {item.type === "header" ? (
                    <h2 className="text-ios-caption1 m-0 bg-bg px-4 pt-3 pb-1.5 font-semibold tracking-[0.06em] text-muted uppercase">
                      {dayLabel(item.day)}
                    </h2>
                  ) : (
                    <MeetingRowView
                      row={item.row}
                      chip={list.chipOf(item.row.gid)}
                      onOpen={() =>
                        void navigate({
                          to: "/meetings/$id",
                          params: { id: item.row.gid },
                        })
                      }
                      onRetry={() => void list.retry(item.row.gid)}
                      onDelete={() => void list.remove(item.row.gid)}
                    />
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
