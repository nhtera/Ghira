// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";
import type { SearchHitView } from "../../bindings";
import { groupHits } from "./group-hits";
import { splitHighlights } from "./highlight";
import { SearchResults } from "./search-results";

afterEach(cleanup);

const hit = (over: Partial<SearchHitView>): SearchHitView => ({
  kind: "segment",
  meeting: "m1",
  meetingTitle: "Client call",
  meetingStartedAt: Date.UTC(2026, 8, 30),
  item: "s1",
  speakerGid: null,
  t0Ms: 65_000,
  t1Ms: 70_000,
  snippet: "Chúng ta nhận diện giọng nói",
  highlights: [[9, 18]],
  exact: true,
  ...over,
});

describe("splitHighlights", () => {
  it("splits on UTF-16 ranges and keeps all text", () => {
    const parts = splitHighlights("Chúng ta nhận diện giọng", [[9, 18]]);
    expect(parts).toEqual([
      { text: "Chúng ta ", mark: false },
      { text: "nhận diện", mark: true },
      { text: " giọng", mark: false },
    ]);
    expect(parts.map((p) => p.text).join("")).toBe("Chúng ta nhận diện giọng");
  });
  it("clips, sorts and merges bad ranges", () => {
    expect(
      splitHighlights("abcdef", [
        [4, 99],
        [1, 3],
        [2, 4],
        [5, 5],
      ]),
    ).toEqual([
      { text: "a", mark: false },
      { text: "bcdef", mark: true },
    ]);
    expect(splitHighlights("abc", [])).toEqual([{ text: "abc", mark: false }]);
  });
  it("counts an emoji as two units, like the store", () => {
    expect(splitHighlights("a😀b", [[1, 3]])).toEqual([
      { text: "a", mark: false },
      { text: "😀", mark: true },
      { text: "b", mark: false },
    ]);
  });
});

describe("groupHits", () => {
  it("groups by meeting in order of first appearance", () => {
    const g = groupHits([hit({ meeting: "a", item: "1" }), hit({ meeting: "b", item: "2" }), hit({ meeting: "a", item: "3" })]);
    expect(g.map((x) => [x.meeting, x.hits.length])).toEqual([
      ["a", 2],
      ["b", 1],
    ]);
  });
});

function view(hits: SearchHitView[], onOpen = vi.fn(), onMore = vi.fn(), hasMore = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <PlatformProvider value="mac">
        <SearchResults hits={hits} hasMore={hasMore} loadingMore={false} onMore={onMore} onOpen={onOpen} />
      </PlatformProvider>
    </QueryClientProvider>,
  );
  return { onOpen, onMore };
}

describe("SearchResults", () => {
  it("renders matches as <mark> text nodes only", () => {
    view([hit({ kind: "note", item: "n1", snippet: "<b>x</b> nhận diện", highlights: [[9, 18]] })]);
    const marks = document.querySelectorAll("mark");
    expect(marks).toHaveLength(1);
    expect(marks[0]!.textContent).toBe("nhận diện");
    // The markup in the text stays text.
    expect(document.querySelector("b")).toBeNull();
    expect(screen.getByText(/<b>x<\/b>/)).toBeTruthy();
  });

  it("opens a segment hit at its time and a note hit in Notes", () => {
    const { onOpen } = view([hit({}), hit({ kind: "note", item: "n1", t0Ms: null, t1Ms: null, snippet: "Quyết định nhận diện", highlights: [[10, 20]] })]);
    const section = screen.getByRole("region", { name: "Client call" });
    const [seg, note] = within(section).getAllByRole("listitem");
    fireEvent.click(within(seg!).getByRole("button"));
    expect(onOpen).toHaveBeenLastCalledWith({ meeting: "m1", tab: "transcript", tMs: 65_000 });
    expect(within(seg!).getByText("1:05")).toBeTruthy();
    fireEvent.click(within(note!).getByRole("button"));
    expect(onOpen).toHaveBeenLastCalledWith({ meeting: "m1", tab: "notes", tMs: null });
  });

  it("offers More results only when there are more", () => {
    const { onMore } = view([hit({})], vi.fn(), vi.fn(), true);
    fireEvent.click(screen.getByRole("button", { name: "More results" }));
    expect(onMore).toHaveBeenCalled();
  });
});
