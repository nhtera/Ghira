// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RelatedHit } from "../../bindings";

const relatedMeetings = vi.hoisted(() => vi.fn());
vi.mock("../../ipc", () => ({ ipc: { commands: { relatedMeetings } } }));

import { NO_FILTERS } from "../library/filters";
import { RelatedSection } from "./related-section";
import { useRelated } from "./use-related";

const hit = (gid: string): RelatedHit => ({ meeting: { meeting: gid, title: `Title ${gid}`, startedAt: Date.UTC(2026, 8, 30) }, t0Ms: 65_000, t1Ms: 70_000, quote: `quote ${gid}` });
const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>{children}</QueryClientProvider>
);

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  relatedMeetings.mockReset();
  relatedMeetings.mockResolvedValue({ status: "ok", data: [hit("a")] });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("useRelated", () => {
  it("waits 600 ms and asks once for the settled text", async () => {
    const { rerender } = renderHook(({ text }) => useRelated(text, NO_FILTERS), { wrapper, initialProps: { text: "" } });
    rerender({ text: "budg" });
    rerender({ text: "budget" });
    await act(async () => void (await vi.advanceTimersByTimeAsync(500)));
    expect(relatedMeetings).not.toHaveBeenCalled();
    await act(async () => void (await vi.advanceTimersByTimeAsync(200)));
    expect(relatedMeetings).toHaveBeenCalledTimes(1);
    expect(relatedMeetings.mock.calls[0]![0]).toBe("budget");
    expect(relatedMeetings.mock.calls[0]![2]).toBe(5);
  });

  it("drops old hits at once when the text changes, before the next debounce", async () => {
    const { result, rerender } = renderHook(({ text }) => useRelated(text, NO_FILTERS), { wrapper, initialProps: { text: "" } });
    rerender({ text: "budget" });
    await act(async () => void (await vi.advanceTimersByTimeAsync(700)));
    await waitFor(() => expect(result.current.map((h) => h.meeting.meeting)).toEqual(["a"]));
    rerender({ text: "budgets" });
    expect(result.current).toEqual([]);
    await act(async () => void (await vi.advanceTimersByTimeAsync(700)));
    expect(relatedMeetings.mock.calls.at(-1)![0]).toBe("budgets");
  });

  it("does not ask for fewer than 3 characters", async () => {
    renderHook(() => useRelated("ab", NO_FILTERS), { wrapper });
    await act(async () => void (await vi.advanceTimersByTimeAsync(1000)));
    expect(relatedMeetings).not.toHaveBeenCalled();
  });
});

describe("RelatedSection", () => {
  it("lists meetings not already in the keyword hits and opens one at its moment", () => {
    const onOpen = vi.fn();
    render(<RelatedSection hits={[hit("a"), hit("b")]} exclude={new Set(["a"])} onOpen={onOpen} />);
    expect(screen.getByText("Related by meaning")).toBeTruthy();
    expect(screen.queryByText("Title a")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Title b/ }));
    expect(onOpen).toHaveBeenCalledWith({ meeting: "b", tab: "transcript", tMs: 65_000 });
  });

  it("is hidden when nothing is left", () => {
    const { container } = render(<RelatedSection hits={[hit("a")]} exclude={new Set(["a"])} onOpen={vi.fn()} />);
    expect(container.textContent).toBe("");
  });
});
