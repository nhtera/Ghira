// SPDX-License-Identifier: Apache-2.0
// The library's "Related by meaning" list obeys the same rules as the keyword hits.
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MeetingRow, RelatedHit } from "../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  listMeetings: vi.fn(),
  searchMeetings: vi.fn(),
  relatedMeetings: vi.fn(),
  listTemplates: vi.fn(),
  listFolders: vi.fn(),
  listTags: vi.fn(),
}));
const navigate = vi.hoisted(() => vi.fn());
const search = vi.hoisted(() => ({ value: {} as { q?: string; folder?: string } }));
vi.mock("../ipc", () => ({ ipc: { commands, onCoreEvent: () => Promise.resolve(() => {}) } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate, useSearch: () => search.value }));
vi.mock("../shell/actions", () => ({ useAppActions: () => ({ startRecording: vi.fn() }) }));

import { renderLive } from "../features/live/test-utils";
import { MeetingsScreen } from "./meetings";

const row = (gid: string, title: string, source: string): MeetingRow => ({
  gid,
  title,
  startedAt: Date.now() - 86_400_000,
  durationMs: 60_000,
  source,
  mode: "call",
  status: "ready",
  transcriptVersion: 2,
  cloudUsed: false,
  consentConfirmed: false,
  template: null,
  people: [],
  job: null,
  folder: null,
  tags: [],
  sourceApp: null,
});
const related = (gid: string, title: string): RelatedHit => ({ meeting: { meeting: gid, title, startedAt: null }, t0Ms: 1000, t1Ms: 2000, quote: `quote ${title}` });

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.listMeetings.mockImplementation((_n: number, offset: number) => ok(offset ? [] : [row("a", "Alpha live", "live"), row("a2", "Alpha two live", "live"), row("b", "Bravo import", "import")]));
  commands.listTemplates.mockResolvedValue([]);
  commands.listFolders.mockReturnValue(ok([{ gid: "f1", name: "Clients", meetings: 1 }]));
  commands.listTags.mockReturnValue(ok([]));
  search.value = {};
  navigate.mockReset();
  commands.searchMeetings.mockReturnValue(
    ok({
      truncated: false,
      hits: [{ kind: "segment", meeting: "a", meetingTitle: "Alpha live", meetingStartedAt: null, item: "s1", speakerGid: null, t0Ms: 0, t1Ms: 1, snippet: "budget talk", highlights: [], exact: true }],
    }),
  );
  commands.relatedMeetings.mockReturnValue(ok([related("a", "Alpha live"), related("a2", "Alpha two live"), related("b", "Bravo import"), related("z", "Zulu unloaded")]));
});
afterEach(cleanup);

describe("MeetingsScreen related list", () => {
  it("leaves out keyword-hit meetings and meetings the filters hide, keeps unloaded ones", async () => {
    const user = userEvent.setup();
    renderLive(<MeetingsScreen />);
    fireEvent.change(await screen.findByRole("searchbox"), { target: { value: "budget" } });
    const section = await screen.findByTestId("related-section", {}, { timeout: 3000 });
    // Alpha has a keyword hit: only in the hits, not repeated as related.
    expect(within(section).queryByText("Alpha live")).toBeNull();
    expect(within(section).getByText("Alpha two live")).toBeTruthy();
    expect(within(section).getByText("Bravo import")).toBeTruthy();
    expect(within(section).getByText("Zulu unloaded")).toBeTruthy();

    // Source = Import hides the live meetings from the list: they leave the related list too.
    await user.click(screen.getByRole("button", { name: "Source" }));
    await user.click(await screen.findByRole("menuitemcheckbox", { name: "Import" }));
    await waitFor(() => expect(within(screen.getByTestId("related-section")).queryByText("Alpha two live")).toBeNull());
    const after = screen.getByTestId("related-section");
    expect(within(after).getByText("Bravo import")).toBeTruthy();
    expect(within(after).getByText("Zulu unloaded")).toBeTruthy();
  }, 15_000);

  it("a folder in the URL that no longer exists is dropped (replace), a known one is kept", async () => {
    search.value = { folder: "gone" };
    renderLive(<MeetingsScreen />);
    await waitFor(() => expect(navigate).toHaveBeenCalled());
    const call = navigate.mock.calls[0]![0] as { replace: boolean; search: (p: object) => object };
    expect(call.replace).toBe(true);
    expect(call.search({ folder: "gone", q: "x" })).toEqual({ folder: undefined, q: "x" });
    cleanup();
    navigate.mockReset();
    search.value = { folder: "f1" };
    renderLive(<MeetingsScreen />);
    await screen.findByRole("searchbox");
    await waitFor(() => expect(commands.listFolders).toHaveBeenCalled());
    expect(navigate).not.toHaveBeenCalled();
  });
});
