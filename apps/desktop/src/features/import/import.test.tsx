// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen, act } from "@testing-library/react";
import { RouterProvider, createMemoryHistory, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import i18n from "i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { ImportUpdate, StagedFile } from "../../bindings";
import { ipc } from "../../ipc";
import { simulateImportDrop } from "../../ipc/mock-review";
import { activeCount, doneCount, importChoice, isImportable, queueReducer, unitsOf, type Queue } from "./import-model";
import { ImportScreen } from "./import-screen";
import { useImportListeners, useImportStore } from "./import-store";

const file = (id: string, over: Partial<StagedFile> = {}): StagedFile => ({
  id,
  name: `${id}.m4a`,
  sizeBytes: 9_000_000,
  durationMs: 660_000,
  channels: 1,
  source: "voiceMemos",
  problems: [],
  duplicateOf: null,
  group: null,
  participant: null,
  title: null,
  startedAt: null,
  ...over,
});
const upd = (id: string, over: Partial<ImportUpdate>): ImportUpdate => ({ id, state: "queued", meeting: null, progress: null, error: null, ...over });

describe("import rules", () => {
  it("unsupported, empty and duplicate files are not importable; very long is", () => {
    expect(isImportable(file("a"))).toBe(true);
    expect(isImportable(file("a", { problems: ["veryLong"] }))).toBe(true);
    for (const p of ["unsupported", "empty", "duplicate", "superseded", "tooManyTracks"] as const) expect(isImportable(file("a", { problems: [p] }))).toBe(false);
  });
  it("the choice: language null = detect; split only with a 2-channel file", () => {
    expect(importChoice("auto", true, [file("a")])).toEqual({ language: null, splitChannels: false });
    expect(importChoice("vi", true, [file("a"), file("b", { channels: 2 })])).toEqual({ language: "vi", splitChannels: true });
    expect(importChoice("en", false, [file("b", { channels: 2 })])).toEqual({ language: "en", splitChannels: false });
  });
});

describe("units: a file, or a Zoom recording's tracks", () => {
  const track = (id: string, group: string | null, participant: string | null) => file(id, { name: `${id}.m4a`, source: "zoom", group, participant });
  it("tracks that share a group are one unit under the group's id; others stay single, in order", () => {
    const staged = [file("a"), track("t1", "g1", "Linh"), file("b"), track("t2", "g1", "Minh"), track("t3", "g1", null)];
    const units = unitsOf(staged);
    expect(units.map((u) => [u.id, u.group, u.files.map((f) => f.id)])).toEqual([
      ["a", false, ["a"]],
      ["g1", true, ["t1", "t2", "t3"]],
      ["b", false, ["b"]],
    ]);
  });
  it("a group left with one track is just a file", () => {
    const units = unitsOf([track("t1", "g1", "Linh")]);
    expect(units.map((u) => [u.id, u.group])).toEqual([["t1", false]]);
  });
});

describe("queue state machine", () => {
  const start = queueReducer(
    {},
    {
      type: "start",
      files: [
        { id: "a", name: "a.m4a" },
        { id: "b", name: "b.m4a" },
      ],
    },
  );
  const step = (q: Queue, u: ImportUpdate) => queueReducer(q, { type: "update", update: u });

  it("queued → decoding (progress) → done with the meeting", () => {
    let q = step(start, upd("a", { state: "decoding", progress: 0.4, meeting: "m1" }));
    expect(q.a).toMatchObject({ state: "decoding", progress: 0.4, meeting: "m1", name: "a.m4a" });
    q = step(q, upd("a", { state: "done", progress: 1, meeting: "m1" }));
    expect(q.a!.state).toBe("done");
    expect(activeCount(q)).toBe(1);
    expect(doneCount(q)).toBe(1);
  });
  it("a late tick can't revive a finished or cancelled file", () => {
    let q = step(start, upd("a", { state: "cancelled" }));
    q = step(q, upd("a", { state: "decoding", progress: 0.5 }));
    expect(q.a!.state).toBe("cancelled");
  });
  it("updates that beat the start (start_import emits before it returns) are kept; only the name is fixed", () => {
    let q = step({}, upd("a", { state: "decoding", progress: 0.3, meeting: "m1" }));
    q = queueReducer(q, {
      type: "start",
      files: [
        { id: "a", name: "a.m4a" },
        { id: "b", name: "b.m4a" },
      ],
    });
    expect(q.a).toMatchObject({ state: "decoding", progress: 0.3, name: "a.m4a" });
    expect(q.b!.state).toBe("queued");
    q = step(q, upd("a", { state: "done", meeting: "m1" }));
    expect(activeCount(q)).toBe(1);
  });
  it("keeps the error of a failed file; clear drops finished ones only", () => {
    let q = step(start, upd("a", { state: "failed", error: "bad audio" }));
    expect(q.a!.error).toBe("bad audio");
    q = queueReducer(q, { type: "clear" });
    expect(Object.keys(q)).toEqual(["b"]);
  });
});

// The app root mounts the listeners.
function Root() {
  useImportListeners();
  return <ImportScreen />;
}

function mount() {
  const root = createRootRoute({ component: Root });
  const detail = createRoute({ getParentRoute: () => root, path: "/meetings/$id/$tab", component: () => null });
  const router = createRouter({ routeTree: root.addChildren([detail]), history: createMemoryHistory({ initialEntries: ["/"] }) });
  return render(
    <PlatformProvider value="mac">
      <ToastProvider label="toasts">
        <RouterProvider router={router} />
      </ToastProvider>
    </PlatformProvider>,
  );
}

describe("ImportScreen", () => {
  beforeEach(() => useImportStore.setState({ staged: [], queue: {} }));
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("shows problems inline, disables Import for blocked files and sends the choice", async () => {
    const files = [
      file("ok", { name: "memo.m4a" }),
      file("zoom", { name: "zoom.m4a", channels: 2, source: "zoom" }),
      file("bad", { name: "bad.xyz", problems: ["unsupported"] }),
      file("dup", { name: "dup.mp3", problems: ["duplicate"], duplicateOf: { meeting: "m9", title: "Client call" } }),
      file("long", { name: "long.wav", problems: ["veryLong"] }),
    ];
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: files });
    const start = vi.spyOn(ipc.commands, "startImport").mockResolvedValue({ status: "ok", data: null });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    expect(await screen.findByText(/Already imported as “Client call”/)).toBeTruthy();
    expect(screen.getByText(/can’t read this file/)).toBeTruthy();
    expect(screen.getByText(/Over 4 hours/)).toBeTruthy();
    // ok + zoom + long can be imported.
    const go = screen.getByRole("button", { name: "Import 3 files" });
    fireEvent.click(screen.getByRole("radio", { name: "Tiếng Việt" }));
    await act(async () => fireEvent.click(go));
    expect(start).toHaveBeenCalledWith(["ok", "zoom", "long"], { language: "vi", splitChannels: true });
    // The started files move to the queue; blocked ones stay staged.
    expect(await screen.findByText("memo.m4a")).toBeTruthy();
    expect(useImportStore.getState().staged.map((f) => f.id)).toEqual(["bad", "dup"]);
  });

  it("follows import updates: waiting, converting %, done with Open notes, failed", async () => {
    useImportStore.getState().dispatch({
      type: "start",
      files: [
        { id: "a", name: "a.m4a" },
        { id: "b", name: "b.m4a" },
      ],
    });
    mount();
    expect(await screen.findByText("2 files importing")).toBeTruthy();
    act(() => {
      useImportStore.getState().dispatch({ type: "update", update: upd("a", { state: "decoding", progress: 0.5, meeting: "m1" }) });
      useImportStore.getState().dispatch({ type: "update", update: upd("b", { state: "failed", error: "unreadable" }) });
    });
    expect(screen.getByText("Converting 50%")).toBeTruthy();
    expect(screen.getByText("Failed: unreadable")).toBeTruthy();
    act(() => useImportStore.getState().dispatch({ type: "update", update: upd("a", { state: "done", progress: 1, meeting: "m1" }) }));
    expect(screen.getByRole("button", { name: "Open notes" })).toBeTruthy();
    expect(screen.queryByText(/files? importing/)).toBeNull();
  });

  it("a drop that arrives as an event is staged (and read again on mount)", async () => {
    const files = [file("d1", { name: "dropped.m4a" })];
    vi.spyOn(ipc.commands, "takeDroppedFiles").mockResolvedValue(files);
    mount();
    await screen.findByRole("button", { name: "Choose files…" });
    await act(async () => {
      simulateImportDrop();
    });
    expect(await screen.findByText("dropped.m4a")).toBeTruthy();
  });

  const zoomTracks = () => [
    file("t1", { name: "audioLinh1111.m4a", source: "zoom", group: "g1", participant: "Linh", title: "Sprint planning", durationMs: 3_540_000 }),
    file("t2", { name: "audioMinh2222.m4a", source: "zoom", group: "g1", participant: "Minh", title: "Sprint planning", durationMs: 3_540_000 }),
    file("t3", { name: "audio_recording_3.m4a", source: "zoom", group: "g1", participant: null, title: "Sprint planning", durationMs: 3_540_000 }),
    file("mix", { name: "audio_only.m4a", source: "zoom", problems: ["superseded"] }),
  ];

  // Files dropped by an earlier test stay in the mock until read.
  beforeEach(() => void vi.spyOn(ipc.commands, "takeDroppedFiles").mockResolvedValue([]));

  it("shows a Zoom recording as one row with its participants, and imports it as one under the group's id", async () => {
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: zoomTracks() });
    const start = vi.spyOn(ipc.commands, "startImport").mockResolvedValue({ status: "ok", data: null });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    expect(await screen.findByText("Zoom recording · 3 participants")).toBeTruthy();
    expect(screen.getByText("Linh")).toBeTruthy();
    expect(screen.getByText("Minh")).toBeTruthy();
    expect(screen.getByText("Participant 3"), "an unnamed track is numbered").toBeTruthy();
    expect(screen.getByText(/knows who said what/)).toBeTruthy();
    // The mixed file is listed as not imported.
    expect(screen.getByText("audio_only.m4a")).toBeTruthy();
    expect(screen.getByText("Won’t be imported")).toBeTruthy();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Import 3 files" })));
    expect(start).toHaveBeenCalledWith(["t1", "t2", "t3"], { language: null, splitChannels: false });
    // The queue has one item named for the meeting, under the group's id; the mixed file stays.
    expect(Object.keys(useImportStore.getState().queue)).toEqual(["g1"]);
    expect(useImportStore.getState().queue.g1!.name).toBe("Sprint planning");
    expect(useImportStore.getState().staged.map((f) => f.id)).toEqual(["mix"]);
  });

  it("removing a participant keeps the rest as a group; the last two can't be split from it into singles", async () => {
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: zoomTracks() });
    vi.spyOn(ipc.commands, "unstageFiles").mockResolvedValue([]);
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    fireEvent.click(await screen.findByRole("button", { name: "Remove Minh" }));
    expect(await screen.findByText("Zoom recording · 2 participants")).toBeTruthy();
    expect(screen.queryByText("Minh")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Remove Linh" }));
    // One track left: an ordinary file row.
    expect(await screen.findByText("audio_recording_3.m4a")).toBeTruthy();
    expect(screen.queryByText(/\d participants?$/)).toBeNull();
  });

  it("a recording imported before is blocked as a whole", async () => {
    const dup = zoomTracks().map((f) => (f.group ? { ...f, problems: ["duplicate" as const], duplicateOf: { meeting: "m9", title: "Sprint planning" } } : f));
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: dup });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    expect(await screen.findByText(/Already imported as “Sprint planning”/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Import 0 files" }).hasAttribute("disabled")).toBe(true);
  });

  it("a code from the core is shown in words", async () => {
    // The words for the core's codes are in the locale files once merged.
    i18n.addResourceBundle("en", "translation", { import: { errors: { tooManyTracks: "A recording can have up to 49 participant tracks." } } }, true, true);
    useImportStore.getState().dispatch({ type: "start", files: [{ id: "g1", name: "Sprint planning" }] });
    mount();
    act(() => useImportStore.getState().dispatch({ type: "update", update: upd("g1", { state: "failed", error: "tooManyTracks" }) }));
    expect(await screen.findByText(/up to 49/)).toBeTruthy();
  });

  const words = () =>
    i18n.addResourceBundle(
      "en",
      "translation",
      {
        import: {
          problems: { superseded: "The Zoom participant tracks are imported instead.", tooManyTracks: "More than 49 participant tracks. Import them separately." },
          group: { separately: "Import tracks separately" },
        },
      },
      true,
      true,
    );

  it("when the last track goes, the core hands the mixed recording back", async () => {
    words();
    const tracks = zoomTracks().slice(0, 2);
    const mixed = zoomTracks()[3]!;
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: [...tracks, mixed] });
    vi.spyOn(ipc.commands, "unstageFiles").mockImplementation(async (ids) => (ids.includes("t2") ? [{ ...mixed, problems: [] }] : []));
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    expect(await screen.findByText("The Zoom participant tracks are imported instead.")).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: "Remove Linh" }));
    // One track left: a file row of its own; removing it removes the last track.
    fireEvent.click(await screen.findByRole("button", { name: "Remove audioMinh2222.m4a" }));
    // The mixed file is a normal row again: nothing says it is left out.
    await act(async () => {});
    expect(screen.queryByText("The Zoom participant tracks are imported instead.")).toBeNull();
    expect(screen.getByRole("button", { name: "Import 1 file" })).toBeTruthy();
  });

  it("'Import tracks separately' turns the group into files and brings the mixed one back", async () => {
    words();
    const tracks = zoomTracks();
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: tracks });
    const apart = vi.spyOn(ipc.commands, "importTracksSeparately").mockResolvedValue({
      status: "ok",
      data: [...tracks.slice(0, 3).map((f) => ({ ...f, group: null, participant: null })), { ...tracks[3]!, problems: [] }],
    });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    fireEvent.click(await screen.findByRole("button", { name: "Import tracks separately" }));
    expect(apart).toHaveBeenCalledWith("g1");
    expect(await screen.findByRole("button", { name: "Import 4 files" })).toBeTruthy();
    expect(screen.queryByText(/Zoom recording · \d+ participants/)).toBeNull();
  });

  it("a recording over the track limit says so and can only go apart", async () => {
    words();
    const big = zoomTracks()
      .slice(0, 3)
      .map((f) => ({ ...f, problems: ["tooManyTracks" as const] }));
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: big });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    expect(await screen.findByText("More than 49 participant tracks. Import them separately.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Import 0 files" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "Import tracks separately" })).toBeTruthy();
  });

  it("each track shows its own problems", async () => {
    const tracks = zoomTracks().slice(0, 3);
    tracks[1] = { ...tracks[1]!, problems: ["veryLong"] };
    tracks[2] = { ...tracks[2]!, problems: ["unsupported"] };
    vi.spyOn(ipc.commands, "pickImportFiles").mockResolvedValue({ status: "ok", data: tracks });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Choose files…" }));
    expect(await screen.findByText(/Over 4 hours/)).toBeTruthy();
    expect(screen.getByText(/can’t read this file/)).toBeTruthy();
  });
});
