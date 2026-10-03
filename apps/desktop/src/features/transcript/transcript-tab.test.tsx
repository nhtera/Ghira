// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { MeetingDetail, MeetingSpeaker, MeetingTranscript, SegmentView } from "../../bindings";
import { ipc } from "../../ipc";
import { usePlayer } from "../../state/player";
import { TranscriptTab } from "./transcript-tab";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const speaker = (gid: string, number: number, over: Partial<MeetingSpeaker> = {}): MeetingSpeaker => ({
  gid,
  name: null,
  number,
  colorSlot: number,
  isMe: false,
  notPerson: false,
  lines: 1,
  sampleT0Ms: null,
  sampleT1Ms: null,
  suggestion: null,
  ...over,
});
const speakers = [speaker("a", 1, { name: "Linh" }), speaker("b", 2)];

const seg = (i: number, spk: string, text: string, over: Partial<SegmentView> = {}): SegmentView => ({
  gid: `s${i}`,
  speakerGid: spk,
  t0Ms: i * 10_000,
  t1Ms: i * 10_000 + 8_000,
  text,
  language: "vi",
  confidence: 0.9,
  edited: false,
  overlap: false,
  words: text.split(" ").map((_, k) => ({ t0Ms: i * 10_000 + k * 1000, t1Ms: i * 10_000 + k * 1000 + 900, confidence: i === 0 && k === 1 ? 0.3 : 0.95 })),
  ...over,
});

const data: MeetingTranscript = {
  version: 2,
  segments: [seg(0, "a", "Xin chào cả nhà"), seg(1, "a", "Mô hình nhận diện tốt"), seg(2, "b", "Nhận diện người nói chưa ổn")],
  marks: [{ tMs: 21_000, tag: "decision" }],
  topics: [{ title: "Opening", tMs: 0 }],
};

const detail = { gid: "m1", durationMs: 40_000, audioAvailable: true, speakers } as unknown as MeetingDetail;

function mount(transcript = data, startAtMs?: number) {
  vi.spyOn(ipc.commands, "meetingTranscript").mockResolvedValue({ status: "ok", data: transcript });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <PlatformProvider value="win">
        <ToastProvider label="Notifications">
          <TranscriptTab meeting="m1" detail={detail} startAtMs={startAtMs} />
        </ToastProvider>
      </PlatformProvider>
    </QueryClientProvider>,
  );
  return client;
}

// happy-dom has no layout: give every element a height so the virtualizer lays rows out.
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(80);
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
});
beforeEach(() => usePlayer.setState({ currentMs: 0, playing: false, src: null, seekRequest: null }));

describe("TranscriptTab", () => {
  it("groups lines by speaker with topic headers, names and the mark", async () => {
    mount();
    const groups = await screen.findAllByTestId("transcript-group");
    expect(groups).toHaveLength(2);
    expect(within(groups[0]!).getByText("Linh")).toBeTruthy();
    expect(within(groups[0]!).getAllByText(/nhận diện|chào/).length).toBeGreaterThan(0);
    expect(within(groups[1]!).getByText("Speaker 2")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Opening" })).toBeTruthy();
    expect(screen.getByTestId("mark").textContent).toContain("Decision");
    expect(screen.getAllByText(/Low confidence/).length).toBeGreaterThan(0);
  });

  it("clicking a line or a group time plays from there", async () => {
    mount();
    fireEvent.click(await screen.findByText("tốt"));
    await waitFor(() => expect(usePlayer.getState().seekRequest).toMatchObject({ ms: 14_000, play: true }));
    fireEvent.click(screen.getAllByRole("button", { name: "Play from 0:20" })[0]!);
    expect(usePlayer.getState().seekRequest).toMatchObject({ ms: 20_000, play: true });
  });

  it("a double-click edits without playing", async () => {
    mount();
    const word = await screen.findByText("Xin");
    fireEvent.click(word, { detail: 1 });
    fireEvent.click(word, { detail: 2 });
    fireEvent.doubleClick(word);
    await new Promise((r) => setTimeout(r, 350));
    expect(usePlayer.getState().seekRequest).toBeNull();
    expect(screen.getByRole("textbox", { name: "Edit text" })).toBeTruthy();
  });

  it("opened at a time (search hit) it scrolls to that line even without audio", async () => {
    const into = vi.fn();
    HTMLElement.prototype.scrollIntoView = into;
    mount(data, 25_000);
    await waitFor(() => expect(into).toHaveBeenCalled());
    expect((into.mock.contexts as HTMLElement[]).some((el) => el.dataset.seg === "2")).toBe(true);
    expect(usePlayer.getState().seekRequest).toBeNull(); // no audio yet: nothing to seek
    act(() => usePlayer.setState({ meeting: "m1", src: "x" }));
    await waitFor(() => expect(usePlayer.getState().seekRequest).toMatchObject({ ms: 25_000 }));
  });

  it("finds without accents and steps through the matches", async () => {
    mount();
    const box = await screen.findByRole("searchbox");
    fireEvent.change(box, { target: { value: "nhan dien" } });
    expect(screen.getByRole("status").textContent).toBe("1 of 2");
    expect(document.querySelectorAll("mark")).toHaveLength(4); // two words each, in two lines
    fireEvent.keyDown(box, { key: "Enter" });
    expect(screen.getByRole("status").textContent).toBe("2 of 2");
    expect([...document.querySelectorAll("mark[data-current=true]")].map((m) => m.textContent).join(" ")).toBe("Nhận diện");
    fireEvent.change(box, { target: { value: "zzz" } });
    expect(screen.getByRole("status").textContent).toBe("No matches");
  });

  it("highlights the spoken word only while playing", async () => {
    mount();
    await screen.findAllByTestId("transcript-group");
    act(() => usePlayer.setState({ currentMs: 1500, playing: false, src: "x" }));
    expect(document.querySelector("[data-active=true]")).toBeNull();
    expect(document.querySelector("[data-playing=true]")).not.toBeNull();
    act(() => usePlayer.setState({ playing: true }));
    expect(document.querySelector("[data-active=true]")?.textContent).toContain("chào");
  });

  it("edits a line and marks it edited", async () => {
    const save = vi.spyOn(ipc.commands, "updateSegmentText").mockResolvedValue({ status: "ok", data: null });
    mount();
    fireEvent.doubleClick(await screen.findByText("Xin"));
    const box = screen.getByRole("textbox", { name: "Edit text" });
    fireEvent.change(box, { target: { value: "Xin chào mọi người" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith("m1", "s0", "Xin chào mọi người"));
  });

  it("moves a line to another speaker", async () => {
    const set = vi.spyOn(ipc.commands, "setSegmentSpeaker").mockResolvedValue({ status: "ok", data: null });
    mount();
    const lines = await screen.findAllByRole("button", { name: "Line actions" });
    fireEvent.keyDown(lines[2]!, { key: "Enter" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Change speaker" }));
    const picker = screen.getByRole("group", { name: "Move this line to" });
    fireEvent.click(within(picker).getByRole("button", { name: /Linh/ }));
    await waitFor(() => expect(set).toHaveBeenCalledWith("m1", "s2", "a"));
  });

  it("shows why when a save fails", async () => {
    vi.spyOn(ipc.commands, "updateSegmentText").mockResolvedValue({ status: "error", error: "disk full" });
    mount();
    fireEvent.doubleClick(await screen.findByText("Xin"));
    fireEvent.change(screen.getByRole("textbox", { name: "Edit text" }), { target: { value: "đổi" } });
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Edit text" }), { key: "Enter", ctrlKey: true });
    expect(await screen.findByText(/disk full/)).toBeTruthy();
  });

  const overlapped: MeetingTranscript = {
    version: 2,
    segments: [seg(0, "a", "Xin chào cả nhà", { overlap: true, t1Ms: 14_000 }), seg(1, "b", "Cho tôi nói với", { t0Ms: 9_000, t1Ms: 16_000, overlap: true }), seg(3, "a", "Tiếp theo nhé")],
    marks: [],
    topics: [],
  };

  it("marks an overlapped line with its hint and mutes its text", async () => {
    // A flagged line nobody overlaps in time stays a plain paragraph, with the full hint.
    mount({ ...overlapped, segments: [seg(0, "a", "Xin chào cả nhà", { overlap: true }), seg(1, "b", "Tiếp theo nhé")] });
    const tags = await screen.findAllByTestId("overlap-tag");
    expect(tags).toHaveLength(1);
    expect(tags[0]!.getAttribute("title")).toBe("Two people spoke at once here, so some words may be wrong.");
    expect(screen.queryByTestId("transcript-stack")).toBeNull();
    expect(document.querySelector('[data-seg="0"]')?.getAttribute("data-overlap")).toBe("true");
    expect(document.querySelector('[data-seg="1"]')?.getAttribute("data-overlap")).toBeNull();
  });

  it("stacks the flagged overlapping lines in one bracket, each line still editable", async () => {
    vi.spyOn(ipc.commands, "updateSegmentText").mockResolvedValue({ status: "ok", data: null });
    mount(overlapped);
    const stack = await screen.findByTestId("transcript-stack");
    expect(stack.getAttribute("aria-label")).toBe("Talking over each other");
    expect(within(stack).getAllByTestId("transcript-group")).toHaveLength(2);
    expect(within(stack).getByText("Cho")).toBeTruthy();
    // The third line is outside the bracket.
    expect(within(stack).queryByText("Tiếp")).toBeNull();
    // The stack says what it is once: its lines carry the short label only.
    const tags = within(stack).getAllByTestId("overlap-tag");
    expect(tags[0]!.getAttribute("title")).toBe("Two people spoke at once here, so some words may be wrong.");
    expect(tags.slice(1).every((t) => t.getAttribute("title") === null)).toBe(true);
    fireEvent.doubleClick(within(stack).getByText("Cho"));
    expect(within(stack).getByRole("textbox", { name: "Edit text" })).toBeTruthy();
  });

  it("find reaches lines inside a stack", async () => {
    mount(overlapped);
    await screen.findByTestId("transcript-stack");
    fireEvent.change(screen.getByRole("searchbox", { name: "Find in transcript" }), { target: { value: "noi voi" } });
    await waitFor(() => expect(within(screen.getByTestId("transcript-stack")).getAllByText("nói", { exact: false }).length).toBeGreaterThan(0));
    expect(document.querySelector("mark")).toBeTruthy();
  });
});
