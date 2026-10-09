// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MeetingSpeaker } from "../../bindings";
import { notesTree } from "@ghi/ui";

const { notesToTree } = notesTree;
type TreeBlock = notesTree.TreeBlock;
import { MindMap } from "./mind-map";
import { extraBlocks } from "./mind-map-tab";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
// jsdom has no layout: give the map a size so Fit has something to fit to.
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1000);
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
});

const b = (gid: string, text: string, at = 1000): TreeBlock => ({ gid, text, citations: [{ t0Ms: at, t1Ms: at + 1000 }] });
const speakers = [{ gid: "s1", name: "Sarah", number: 1, colorSlot: 2, isMe: false, notPerson: false, lines: 1, sampleT0Ms: null, sampleT1Ms: null }] as unknown as MeetingSpeaker[];
const inputBase = {
  title: "Client call",
  titles: { summary: "Summary", decisions: "Decisions", proposed: "Proposed", actions: "Action items", questions: "Open questions", topics: "Topics", answers: "Saved from Ask", marked: "Marked", other: "Other" },
  tldr: [b("t1", "We ship on the 12th.")],
  sections: [],
  decisions: [b("d1", "Rename ships in beta.", 5000), b("d2", "Consent first.", 6000)],
  actions: [{ ...b("a1", "Send the deck", 7000), ownerSpeakerGid: "s1", dueText: null, done: false }],
  questions: [],
  topics: [],
  proposals: [b("p1", "Schedule a follow-up", 8000)],
  covered: new Set(["d1"]),
};
const root = notesToTree(inputBase);

const mount = (over: Partial<Parameters<typeof MindMap>[0]> = {}) => {
  const onActivate = vi.fn();
  const onCopy = vi.fn();
  render(<MindMap root={root} speakers={speakers} onActivate={onActivate} onCopy={onCopy} {...over} />);
  return { onActivate, onCopy };
};
const tree = () => screen.getByRole("tree", { name: "Mind map outline" });
const item = (name: RegExp | string) => within(tree()).getByRole("treeitem", { name });
const nodes = () => document.querySelectorAll("[data-node]");

describe("MindMap", () => {
  it("draws every node and mirrors them in the tree list", () => {
    mount();
    expect(nodes()).toHaveLength(1 + 4 + 5); // root + sections + leaves
    expect(within(tree()).getAllByRole("treeitem")).toHaveLength(10);
    expect(item("Client call").getAttribute("aria-level")).toBe("1");
    expect(item(/^Rename ships in beta\./).getAttribute("aria-level")).toBe("3");
    // owner, proposed and covered-moment cues are text, not color alone
    expect(item(/Send the deck/).textContent).toContain("Sarah");
    expect(item(/Schedule a follow-up/).textContent).toContain("Proposed");
    expect(item(/Rename ships/).textContent).toContain("Covers a moment you marked");
    expect(document.querySelector('[data-node="p1"]')?.textContent).toContain("Proposed");
  });

  it("walks with the arrow keys and plays a leaf with Enter", () => {
    const { onActivate } = mount();
    const tr = tree();
    // The roving stop starts on the first section.
    const first = item("Summary");
    expect(first.getAttribute("tabindex")).toBe("0");
    act(() => first.focus());
    fireEvent.keyDown(tr, { key: "ArrowRight" }); // into the section
    expect(document.activeElement).toBe(item("We ship on the 12th."));
    fireEvent.keyDown(tr, { key: "ArrowDown" });
    expect(document.activeElement).toBe(item("Decisions"));
    fireEvent.keyDown(tr, { key: "ArrowRight" });
    fireEvent.keyDown(tr, { key: "Enter" });
    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(onActivate.mock.calls[0]![0]).toMatchObject({ id: "d1" });
    expect(onActivate.mock.calls[0]![1]).toBe(false);
    fireEvent.keyDown(tr, { key: "Enter", ctrlKey: true });
    expect(onActivate.mock.calls[1]![1]).toBe(true);
    fireEvent.keyDown(tr, { key: "ArrowLeft" }); // back to the section
    expect(document.activeElement).toBe(item("Decisions"));
  });

  it("collapses a section from the keyboard and from the map", () => {
    mount();
    const tr = tree();
    act(() => item("Decisions").focus());
    fireEvent.keyDown(tr, { key: "ArrowLeft" });
    expect(item("Decisions").getAttribute("aria-expanded")).toBe("false");
    expect(within(tr).queryByRole("treeitem", { name: "Consent first." })).toBeNull();
    expect(document.querySelector('[data-node="d2"]')).toBeNull();
    fireEvent.keyDown(tr, { key: "ArrowRight" });
    expect(item("Decisions").getAttribute("aria-expanded")).toBe("true");
    // a click on the section node on the map toggles it too
    fireEvent.click(document.querySelector('[data-node="sec:decisions"]')!);
    expect(document.querySelector('[data-node="d1"]')).toBeNull();
    expect(item("Decisions").getAttribute("aria-expanded")).toBe("false");
  });

  it("a click on a leaf plays, a Cmd/Ctrl-click shows it in the transcript; the root does nothing", () => {
    const { onActivate } = mount();
    fireEvent.click(document.querySelector('[data-node="a1"]')!);
    expect(onActivate).toHaveBeenLastCalledWith(expect.objectContaining({ id: "a1" }), false);
    fireEvent.click(document.querySelector('[data-node="a1"]')!, { metaKey: true });
    expect(onActivate).toHaveBeenLastCalledWith(expect.objectContaining({ id: "a1" }), true);
    onActivate.mockClear();
    fireEvent.click(document.querySelector('[data-node="root"]')!);
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("a drag pans and does not count as a click", () => {
    const { onActivate } = mount();
    const canvas = screen.getByTestId("mind-map-canvas");
    const before = canvas.style.transform;
    const leaf = document.querySelector('[data-node="a1"]')!;
    fireEvent.pointerDown(leaf, { button: 0, clientX: 100, clientY: 100 });
    fireEvent(window, new MouseEvent("pointermove", { clientX: 160, clientY: 130 }));
    fireEvent(window, new MouseEvent("pointerup"));
    fireEvent.click(leaf);
    expect(canvas.style.transform).not.toBe(before);
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("zooms with the buttons and the + - keys, and 0 / Fit return to the fitted view", () => {
    mount();
    const pct = () => Number.parseInt(screen.getByText(/%$/).textContent!, 10);
    const view = screen.getByRole("group", { name: "Mind map of the notes" });
    const fitted = pct();
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(pct()).toBeGreaterThan(fitted);
    fireEvent.click(screen.getByRole("button", { name: "Fit" }));
    expect(pct()).toBe(fitted);
    fireEvent.keyDown(view, { key: "+" });
    const zoomedIn = pct();
    expect(zoomedIn).toBeGreaterThan(fitted);
    fireEvent.keyDown(view, { key: "-" });
    expect(pct()).toBeLessThan(zoomedIn);
    fireEvent.keyDown(view, { key: "-" });
    fireEvent.keyDown(view, { key: "0" });
    expect(pct()).toBe(fitted);
    // with a modifier the keys are the browser's (Cmd/Ctrl +/-/0)
    fireEvent.keyDown(view, { key: "+", ctrlKey: true });
    fireEvent.keyDown(view, { key: "+", metaKey: true });
    expect(pct()).toBe(fitted);
  });

  it("wheel zooms the map but leaves the list's own scrolling alone", () => {
    mount();
    const view = screen.getByRole("group", { name: "Mind map of the notes" });
    const onMap = new WheelEvent("wheel", { deltaY: -200, cancelable: true, bubbles: true });
    screen.getByTestId("mind-map-canvas").dispatchEvent(onMap);
    expect(onMap.defaultPrevented).toBe(true);
    const inList = new WheelEvent("wheel", { deltaY: -200, cancelable: true, bubbles: true });
    tree().dispatchEvent(inList);
    expect(inList.defaultPrevented).toBe(false);
    void view;
  });

  it("the arrow keys on the map itself go into the tree", () => {
    mount();
    const view = screen.getByRole("group", { name: "Mind map of the notes" });
    act(() => view.focus());
    fireEvent.keyDown(view, { key: "ArrowDown" });
    expect(document.activeElement).toBe(item("Summary"));
  });

  it("marks a done action in the list", () => {
    const done = notesToTree({ ...inputBase, actions: [{ ...b("a9", "Ship it", 1000), ownerSpeakerGid: null, dueText: null, done: true }] });
    render(<MindMap root={done} speakers={speakers} onActivate={vi.fn()} onCopy={vi.fn()} />);
    expect(item(/Ship it/).textContent).toContain("Done");
  });

  it("does not animate view changes under reduced motion", () => {
    const real = window.matchMedia;
    const mm = (q: string) => ({ matches: q.includes("reduce"), media: q, addEventListener: () => undefined, removeEventListener: () => undefined }) as unknown as MediaQueryList;
    window.matchMedia = mm;
    try {
      mount();
      fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
      expect(screen.getByTestId("mind-map-canvas").getAttribute("class") ?? "").not.toContain("transition-transform");
    } finally {
      window.matchMedia = real;
    }
    cleanup();
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByTestId("mind-map-canvas").getAttribute("class") ?? "").toContain("transition-transform");
  });

  it("Copy as outline calls back", () => {
    const { onCopy } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Copy as outline" }));
    expect(onCopy).toHaveBeenCalledTimes(1);
  });
});

describe("extraBlocks", () => {
  const blk = (kind: string, origin: "ai" | "user" = "ai") => ({ gid: kind, kind, origin, text: kind, pinned: false, citations: [] }) as never;
  it("keeps only kinds the Notes tab does not place (proposals are a section of their own now)", () => {
    const r = extraBlocks([blk("tldr"), blk("decision"), blk("section:risks"), blk("enhanced:x"), blk("note", "user"), blk("proposal"), blk("future-kind"), blk("mystery", "user")]);
    expect(r.other.map((x) => x.kind)).toEqual(["future-kind"]);
  });
});
