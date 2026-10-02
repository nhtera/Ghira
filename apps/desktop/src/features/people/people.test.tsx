// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";
import type { PeopleList, PersonDetail, PersonRow } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const commands = vi.hoisted(() => ({
  listPeople: vi.fn(),
  personDetail: vi.fn(),
  mergePeople: vi.fn(),
  deleteVoiceData: vi.fn(),
  removePersonName: vi.fn(),
  voiceStatus: vi.fn(),
  issueAudioSample: vi.fn(),
  enrollVoiceCancel: vi.fn(),
}));
const navigate = vi.hoisted(() => vi.fn());
vi.mock("../../ipc", () => ({ ipc: { commands, audioUrl: (t: string) => `ghi-audio://${t}` } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import { renderLive } from "../live/test-utils";
import { PeopleBody } from "./people-screen";

const person = (gid: string, name: string, over: Partial<PersonRow> = {}): PersonRow => ({
  gid,
  name,
  isMe: false,
  colorSlot: 2,
  meetings: 3,
  openActions: 1,
  lastMetMs: Date.UTC(2026, 8, 30),
  voice: { kind: "none", atMs: null },
  ...over,
});
const me = person("me", "", { isMe: true, meetings: 40, openActions: 0, voice: { kind: "self", atMs: Date.UTC(2026, 7, 1) } });
const linh = person("linh", "Linh", { voice: { kind: "agreed", atMs: Date.UTC(2026, 7, 12) } });
const minh = person("minh", "Minh");
const anh = person("anh", "Anh");
const list = (people: PersonRow[]): PeopleList => ({ people, thirdParty: false });
const detailOf = (p: PersonRow): PersonDetail => ({
  person: p,
  meetings: [{ gid: "m1", title: "Product sync", startedAt: Date.UTC(2026, 8, 30), durationMs: 60_000 }],
  openActions: [{ gid: "a1", meetingGid: "m1", meetingTitle: "Product sync", text: "Send the mockups", due: null, dueText: "Fri" }],
  samples: p.voice.kind === "none" ? [] : [{ meetingGid: "m1", meetingTitle: "Product sync", t0Ms: 65_000, t1Ms: 68_000, track: null }],
});

const show = () =>
  renderLive(
    <PlatformProvider value="mac">
      <PeopleBody />
    </PlatformProvider>,
  );
const open = async (name: RegExp | string) => userEvent.setup().click(await screen.findByRole("button", { name }));

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  navigate.mockReset();
  commands.listPeople.mockReturnValue(ok(list([me, linh, minh, anh])));
  commands.personDetail.mockImplementation((gid: string) => ok(detailOf([me, linh, minh, anh].find((p) => p.gid === gid)!)));
  commands.voiceStatus.mockReturnValue(ok({ modelReady: true, meProfile: { atMs: Date.UTC(2026, 7, 1), samples: 6 }, enrolling: false }));
  commands.deleteVoiceData.mockReturnValue(ok(null));
  commands.removePersonName.mockReturnValue(ok(3));
  commands.mergePeople.mockReturnValue(ok(null));
  commands.issueAudioSample.mockReturnValue(err("no audio"));
});
afterEach(cleanup);

describe("People", () => {
  it("lists Me first, with meetings, open items and the voice state in words", async () => {
    show();
    const nav = await screen.findByRole("navigation", { name: "People" });
    const rows = within(nav).getAllByRole("button");
    expect(rows[0]!.textContent).toContain("Me");
    expect(rows[1]!.textContent).toContain("Linh");
    expect(within(rows[1]!).getByText(/3 meetings · 1 open/)).toBeTruthy();
    // Voice state is an icon plus text (not color alone).
    expect(within(rows[0]!).getByText("Your voice · enrolled")).toBeTruthy();
    expect(within(rows[1]!).getByText(/Voice profile · agreed/)).toBeTruthy();
    expect(within(rows[2]!).getByText("No voice profile")).toBeTruthy();
    // Other people's voices are off: no unknown-voices queue.
    expect(screen.queryByText("Unknown voices")).toBeNull();
  });

  it("shows the empty state when nobody has been named", async () => {
    commands.listPeople.mockReturnValue(ok(list([{ ...me, meetings: 0 }])));
    show();
    expect(await screen.findByText("People appear after your first meeting")).toBeTruthy();
  });

  it("a person's page: samples, open items, meetings together; a meeting opens", async () => {
    show();
    await open(/Linh/);
    expect(await screen.findByRole("heading", { name: "Linh" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Voice samples" })).toBeTruthy();
    expect(screen.getByText("Send the mockups")).toBeTruthy();
    await userEvent.setup().click(within(screen.getByRole("region", { name: "Meetings together" })).getByRole("button", { name: /Product sync/ }));
    expect(navigate).toHaveBeenCalledWith({ to: "/meetings/$id/$tab", params: { id: "m1", tab: "notes" } });
  });

  it("playing a sample asks for exactly that span and track", async () => {
    show();
    await open(/Linh/);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Play 3 s sample" }));
    expect(commands.issueAudioSample).toHaveBeenCalledWith("m1", 65_000, 68_000, null);
  });

  it("Delete voice data changes only the voice side", async () => {
    show();
    await open(/Linh/);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Delete voice data…" }));
    const confirm = screen.getByRole("alertdialog");
    expect(confirm.textContent).toContain("Their name stays in 3 meetings");
    // The name confirm is a different control and stays closed.
    expect(screen.getAllByRole("alertdialog")).toHaveLength(1);
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(commands.deleteVoiceData).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Delete voice data…" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete voice data" }));
    await waitFor(() => expect(commands.deleteVoiceData).toHaveBeenCalledExactlyOnceWith("linh"));
    expect(commands.removePersonName).not.toHaveBeenCalled();
    expect(await screen.findByText(/Voice data deleted/)).toBeTruthy();
  });

  it("Remove name from notes changes only the name side and says how many meetings", async () => {
    show();
    await open(/Linh/);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Remove name from notes…" }));
    const confirm = screen.getByRole("alertdialog");
    expect(confirm.textContent).toContain("Replace “Linh” with “Speaker N” in 3 meetings");
    await user.click(within(confirm).getByRole("button", { name: "Remove name" }));
    await waitFor(() => expect(commands.removePersonName).toHaveBeenCalledExactlyOnceWith("linh"));
    expect(commands.deleteVoiceData).not.toHaveBeenCalled();
    expect(await screen.findByText("Name removed in 3 meetings.")).toBeTruthy();
  });

  it("a person without a voice profile has no delete-voice action; Me has no remove-name or merge", async () => {
    show();
    await open(/Minh/);
    await screen.findByRole("heading", { name: "Minh" });
    expect(screen.queryByRole("button", { name: "Delete voice data…" })).toBeNull();
    expect(screen.getByText(/No voice data is stored for Minh/)).toBeTruthy();
    await userEvent.setup().click(within(screen.getByRole("navigation", { name: "People" })).getAllByRole("button")[0]!);
    await screen.findByRole("heading", { name: "Me" });
    expect(screen.queryByRole("button", { name: "Remove name from notes…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Merge with…" })).toBeNull();
    expect(screen.getByRole("button", { name: "Record your voice again…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Delete voice data…" })).toBeTruthy();
  });

  it("merging asks first, then merges; people with a voice profile can't be merged", async () => {
    show();
    await open(/Minh/);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Merge with…" }));
    // Linh has a voice profile and other people's profiles are off: not offered as a target.
    expect((await screen.findByRole("menuitem", { name: "Linh" })).getAttribute("aria-disabled")).toBe("true");
    expect(screen.getByText("People with a voice profile can’t be merged in this version.")).toBeTruthy();
    await user.click(screen.getByRole("menuitem", { name: "Anh" }));
    const confirm = await screen.findByRole("alertdialog");
    expect(confirm.textContent).toContain("Merge Minh into Anh?");
    expect(commands.mergePeople).not.toHaveBeenCalled();
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(commands.mergePeople).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Merge with…" }));
    await user.click(await screen.findByRole("menuitem", { name: "Anh" }));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(commands.mergePeople).toHaveBeenCalledExactlyOnceWith("minh", "anh"));
    expect(await screen.findByText("Merged Minh into Anh")).toBeTruthy();
  });

  it("someone with a voice profile has the whole Merge button off", async () => {
    show();
    await open(/Linh/);
    await screen.findByRole("heading", { name: "Linh" });
    expect((screen.getByRole("button", { name: "Merge with…" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("a refusal code is shown as a sentence (busyRecording, thirdPartyOff)", async () => {
    const user = userEvent.setup();
    for (const [code, text] of [
      ["busyRecording", "This waits until the recording stops."],
      ["thirdPartyOff", "Other people’s voice profiles are off."],
    ] as const) {
      commands.mergePeople.mockReturnValue(err(code));
      const view = show();
      await open(/Minh/);
      await user.click(await screen.findByRole("button", { name: "Merge with…" }));
      await user.click(await screen.findByRole("menuitem", { name: "Anh" }));
      await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Merge" }));
      expect(await screen.findByText(text)).toBeTruthy();
      view.unmount();
    }
  });

  it("removing the name of someone without a voice profile selects the first row, not a vanished person", async () => {
    commands.removePersonName.mockReturnValue(ok(2));
    show();
    await open(/Minh/);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Remove name from notes…" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Remove name" }));
    await waitFor(() => expect(commands.removePersonName).toHaveBeenCalledWith("minh"));
    expect(await screen.findByRole("heading", { name: "Me" })).toBeTruthy();
    // No raw code or error for the person who just went away.
    expect(screen.queryByText(/notFound/)).toBeNull();
  });

  it("a failing read is a sentence, not a raw code", async () => {
    commands.listPeople.mockReturnValue(err("storage"));
    show();
    expect(await screen.findByText(/Couldn.t read or write the data/)).toBeTruthy();
  });

  it("playing a sample can be stopped and played again", async () => {
    commands.issueAudioSample.mockReturnValue(ok("tok"));
    show();
    await open(/Linh/);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Play 3 s sample" }));
    await user.click(await screen.findByRole("button", { name: "Stop the sample" }));
    expect(screen.queryByRole("button", { name: "Stop the sample" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Play 3 s sample" }));
    expect(commands.issueAudioSample).toHaveBeenCalledTimes(2);
  });
});
