// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen, act } from "@testing-library/react";
import { RouterProvider, createMemoryHistory, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { ImportUpdate, StagedFile } from "../../bindings";
import { ipc } from "../../ipc";
import { simulateImportDrop } from "../../ipc/mock-review";
import { activeCount, doneCount, importChoice, isImportable, queueReducer, type Queue } from "./import-model";
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
  ...over,
});
const upd = (id: string, over: Partial<ImportUpdate>): ImportUpdate => ({ id, state: "queued", meeting: null, progress: null, error: null, ...over });

describe("import rules", () => {
  it("unsupported, empty and duplicate files are not importable; very long is", () => {
    expect(isImportable(file("a"))).toBe(true);
    expect(isImportable(file("a", { problems: ["veryLong"] }))).toBe(true);
    for (const p of ["unsupported", "empty", "duplicate"] as const) expect(isImportable(file("a", { problems: [p] }))).toBe(false);
  });
  it("the choice: language null = detect; split only with a 2-channel file", () => {
    expect(importChoice("auto", true, [file("a")])).toEqual({ language: null, splitChannels: false });
    expect(importChoice("vi", true, [file("a"), file("b", { channels: 2 })])).toEqual({ language: "vi", splitChannels: true });
    expect(importChoice("en", false, [file("b", { channels: 2 })])).toEqual({ language: "en", splitChannels: false });
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
});
