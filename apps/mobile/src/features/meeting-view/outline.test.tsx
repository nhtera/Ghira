// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { PlatformProvider, type NotesTreeInput } from "@ghi/ui";
import type { MeetingSpeaker } from "../../bindings";
import { Outline } from "./outline";
import { TalkShareBar } from "./talk-share-bar";

afterEach(cleanup);

const wrap = (ui: React.ReactNode, lang: "en" | "vi" = "en") =>
  render(
    <I18nextProvider i18n={initMobileI18n(lang)}>
      <PlatformProvider value="ios">{ui}</PlatformProvider>
    </I18nextProvider>,
  );

const b = (gid: string, text: string, at: number | null = 1000, missing = false) => ({
  gid,
  text,
  citations: at == null ? [] : [{ t0Ms: at, t1Ms: at + 1000, missing }],
});
const input: NotesTreeInput = {
  title: "Product sync",
  titles: { summary: "Summary", decisions: "Decisions", proposed: "Proposed", actions: "Action items", questions: "Open questions", topics: "Topics", marked: "Moments you marked", other: "Other" },
  tldr: [b("t1", "We ship on the 12th.", 90_000)],
  sections: [],
  decisions: [b("d1", "Scope: iPhone only.", 96_000)],
  proposals: [b("p1", "Maybe Android later.", 300_000)],
  actions: [{ ...b("a1", "Send the budget", 105_000), ownerSpeakerGid: "s1", dueText: "Fri", done: false }],
  questions: [b("q1", "Who signs off?", 200_000, true)],
  topics: [],
  other: [b("o1", "A block of a newer kind.", null)],
  covered: new Set(["d1"]),
};
const speakers = [{ gid: "s1", name: "Sarah", number: 1, colorSlot: 2, isMe: false, notPerson: false, lines: 1, sampleT0Ms: null, sampleT1Ms: null }] as unknown as MeetingSpeaker[];

describe("Outline", () => {
  it("is collapsed until opened, then lists every section of the notes", () => {
    wrap(<Outline input={input} speakers={speakers} onPlayAt={vi.fn()} />);
    const toggle = screen.getByRole("button", { name: /^Outline \(\d+\)$/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("Scope: iPhone only.")).toBeNull();
    fireEvent.click(toggle);
    const o = within(screen.getByTestId("outline"));
    for (const h of ["Summary", "Decisions", "Proposed", "Action items", "Open questions", "Other"]) expect(o.getByRole("heading", { name: h })).toBeTruthy();
    // an unknown kind is kept, under Other
    expect(o.getByText("A block of a newer kind.")).toBeTruthy();
  });

  it("marks proposed, owned, due and starred leaves with words, not color", () => {
    wrap(<Outline input={input} speakers={speakers} onPlayAt={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /^Outline/ }));
    expect(screen.getByTestId("proposed-chip").textContent).toBe("Proposed");
    expect(screen.getByText(/Send the budget/).parentElement!.textContent).toContain("Sarah");
    expect(screen.getByText(/Send the budget/).parentElement!.textContent).toContain("Fri");
    expect(screen.getByText("Covers a moment you marked")).toBeTruthy();
  });

  it("a tap on a leaf plays its first citation; one without audio or a missing moment is plain text", () => {
    const play = vi.fn();
    wrap(<Outline input={input} speakers={speakers} onPlayAt={play} />);
    fireEvent.click(screen.getByRole("button", { name: /^Outline/ }));
    fireEvent.click(screen.getByRole("button", { name: "Play from 01:36" }));
    expect(play).toHaveBeenCalledWith(96_000);
    // missing citation (q1) and no citation (o1): not buttons
    expect(screen.queryByRole("button", { name: "Play from 03:20" })).toBeNull();
    expect(screen.getAllByRole("button", { name: /^Play from/ })).toHaveLength(4);
  });

  it("without audio nothing plays", () => {
    wrap(<Outline input={input} speakers={speakers} />);
    fireEvent.click(screen.getByRole("button", { name: /^Outline/ }));
    expect(screen.queryAllByRole("button", { name: /^Play from/ })).toHaveLength(0);
    expect(screen.getByText("Scope: iPhone only.")).toBeTruthy();
  });

  it("an empty notes tree renders nothing", () => {
    wrap(<Outline input={{ ...input, tldr: [], decisions: [], proposals: [], actions: [], questions: [], other: [] }} speakers={speakers} />);
    expect(screen.queryByTestId("outline")).toBeNull();
  });
});

describe("TalkShareBar", () => {
  const spk = (gid: string, name: string | null, slot: number, over: Partial<MeetingSpeaker> = {}) =>
    ({ gid, name, number: slot, colorSlot: slot, isMe: false, notPerson: false, lines: 1, sampleT0Ms: null, sampleT1Ms: null, ...over }) as unknown as MeetingSpeaker;
  const seg = (speakerGid: string | null, t0Ms: number, t1Ms: number) => ({ speakerGid, t0Ms, t1Ms }) as never;

  it("shows each speaker's share with an initial and a name, and what nobody owns as Unassigned", () => {
    wrap(<TalkShareBar segments={[seg("a", 0, 6000), seg("b", 6000, 9000), seg(null, 9000, 10_000)]} speakers={[spk("a", "Linh", 1, { lines: 2 }), spk("b", null, 2)]} />);
    const bar = within(screen.getByTestId("talk-share"));
    expect(bar.getByText(/2 speakers · 4 turns/)).toBeTruthy();
    const entries = bar.getAllByRole("listitem");
    expect(entries.map((e) => e.textContent)).toEqual([expect.stringMatching(/^LLinh60%2 turns$/), expect.stringMatching(/^2Speaker 230%1 turn$/), expect.stringMatching(/^\?Unassigned10%1 turn$/)]);
    expect(entries.reduce((n, e) => n + Number.parseInt(e.querySelector(".font-mono")!.textContent!, 10), 0)).toBe(100);
  });

  it("is left out without any diarized speaker or talk time, and not-a-person speakers do not count", () => {
    const { container } = wrap(<TalkShareBar segments={[seg(null, 0, 4000)]} speakers={[spk("a", "Linh", 1)]} />);
    expect(container.querySelector('[data-testid="talk-share"]')).toBeNull();
    cleanup();
    wrap(<TalkShareBar segments={[seg("m", 0, 4000)]} speakers={[spk("m", "Music", 3, { notPerson: true })]} />);
    expect(screen.queryByTestId("talk-share")).toBeNull();
  });
});
