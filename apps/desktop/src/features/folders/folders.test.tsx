// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FolderRow, MeetingRow, TagRow } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const commands = vi.hoisted(() => ({
  listMeetings: vi.fn(),
  listFolders: vi.fn(),
  listTags: vi.fn(),
  createFolder: vi.fn(),
  createTag: vi.fn(),
  renameFolder: vi.fn(),
  renameTag: vi.fn(),
  deleteFolder: vi.fn(),
  deleteTag: vi.fn(),
  moveToFolder: vi.fn(),
  tagMeetings: vi.fn(),
  untagMeetings: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands, onCoreEvent: () => Promise.resolve(() => {}) } }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, search, ...rest }: { children: unknown; search?: { folder?: string } }) => <a href={`#/meetings?folder=${search?.folder}`} {...(rest as object)}>{children as never}</a>,
  useSearch: () => ({}),
}));

import { renderLive } from "../live/test-utils";
import { BulkOrganize } from "./bulk-organize";
import { FolderTags } from "./folder-tags";
import { OrganizeCard } from "./organize-card";
import { RowChips } from "./row-chips";
import { SidebarFolders } from "./sidebar-folders";
import { choicesFor } from "./tag-combobox";

const folder = (gid: string, name: string, meetings = 0): FolderRow => ({ gid, name, meetings });
const tag = (gid: string, name: string, meetings = 0): TagRow => ({ gid, name, meetings });
const row = (gid: string, over: Partial<MeetingRow> = {}): MeetingRow => ({
  gid,
  title: gid,
  startedAt: 1,
  durationMs: 1,
  source: "live",
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
  summary: null,
  unnamedVoices: 0,
  ...over,
});

let folders: FolderRow[];
let tags: TagRow[];
let rows: MeetingRow[];
beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  folders = [folder("f1", "Clients", 2)];
  tags = [tag("t1", "Họp", 3)];
  rows = [row("m1"), row("m2", { folder: "f1", tags: [{ gid: "t1", name: "Họp" }] })];
  commands.listFolders.mockImplementation(() => ok(folders));
  commands.listTags.mockImplementation(() => ok(tags));
  commands.listMeetings.mockImplementation((_n: number, offset: number) => ok(offset ? [] : rows));
  for (const c of ["renameFolder", "renameTag", "untagMeetings", "tagMeetings", "moveToFolder", "deleteFolder", "deleteTag"] as const) commands[c].mockReturnValue(ok(1));
});
afterEach(cleanup);

describe("choicesFor", () => {
  const list = [tag("t1", "Họp"), tag("t2", "Hộp"), tag("t3", "Planning")];
  const names = (c: ReturnType<typeof choicesFor>) => c.map((x) => (x.kind === "tag" ? x.tag.name : `+${x.name}`));
  it("suggests tags ignoring accents and case", () => {
    expect(names(choicesFor("hop", list, new Set()))).toEqual(["Họp", "Hộp", "+hop"]);
    expect(names(choicesFor("PLAN", list, new Set()))).toEqual(["Planning", "+PLAN"]);
  });
  it("accents count for the exact name: no Create when it exists, even if the meeting has it already", () => {
    expect(names(choicesFor("Họp", list, new Set()))).toEqual(["Họp", "Hộp"]);
    expect(names(choicesFor("họp", list, new Set()))).not.toContain("+họp");
    expect(names(choicesFor("Họp", list, new Set(["t1"])))).not.toContain("+Họp");
  });
  it("a lone accent variant is reused for a name typed without accents", () => {
    expect(names(choicesFor("hop", [tag("t1", "Họp")], new Set()))).toEqual(["Họp"]);
  });
  it("a name typed WITH accents is a new tag even when another accent variant exists", () => {
    // The core reuses a tag only for unaccented input; "Hộp" must not attach "Họp".
    expect(names(choicesFor("Hộp", [tag("t1", "Họp")], new Set()))).toEqual(["Họp", "+Hộp"]);
  });
  it("two accent variants: unaccented input is ambiguous, so it offers Create", () => {
    expect(names(choicesFor("hop", list, new Set())).at(-1)).toBe("+hop");
  });
  it("tags the meeting already has are not offered", () => {
    expect(names(choicesFor("", list, new Set(["t1"])))).toEqual(["Hộp", "Planning"]);
  });
  it("nothing typed offers no Create", () => {
    expect(choicesFor("   ", list, new Set()).some((c) => c.kind === "create")).toBe(false);
  });
});

describe("organizeError", () => {
  it("never shows a raw code", async () => {
    const { organizeError } = await import("./organize");
    const { default: i18n } = await import("i18next");
    for (const code of ["notFound", "storage", "somethingNew"]) {
      const text = organizeError(i18n.t.bind(i18n) as never, code) ?? "";
      expect(text).not.toContain(code);
      expect(text.length).toBeGreaterThan(0);
    }
    expect(organizeError(i18n.t.bind(i18n) as never, "empty")).toBeNull();
  });
});

describe("RowChips", () => {
  it("shows folder, three tags and +N, and the app a file came from", () => {
    const many = ["a", "b", "c", "d", "e"].map((n) => ({ gid: n, name: `tag-${n}` }));
    renderLive(<RowChips row={row("m", { tags: many, sourceApp: "plaud" })} folderName="Clients" />);
    expect(screen.getByText("Clients")).toBeTruthy();
    expect(screen.getByText("tag-c")).toBeTruthy();
    expect(screen.queryByText("tag-d")).toBeNull();
    expect(screen.getByText("+2")).toBeTruthy();
    expect(screen.getByText("Plaud import")).toBeTruthy();
  });
  it("renders nothing for a plain row", () => {
    const { container } = renderLive(<RowChips row={row("m")} />);
    expect(container.textContent).toBe("");
  });
});

describe("FolderTags (meeting header)", () => {
  it("shows the folder and tags; × removes a tag", async () => {
    renderLive(<FolderTags meeting="m2" />);
    expect(await screen.findByText("Clients")).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Remove tag Họp" }));
    await waitFor(() => expect(commands.untagMeetings).toHaveBeenCalledExactlyOnceWith(["m2"], "t1"));
  });

  it("Add tag…: picking an existing tag tags the meeting", async () => {
    renderLive(<FolderTags meeting="m1" />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Add tag…" }));
    await user.type(screen.getByRole("combobox", { name: "Tag name" }), "hop");
    await user.click(await screen.findByRole("option", { name: "Họp" }));
    await waitFor(() => expect(commands.tagMeetings).toHaveBeenCalledExactlyOnceWith(["m1"], "t1"));
    expect(commands.createTag).not.toHaveBeenCalled();
  });

  it("Add tag…: typing a new name offers Create, makes the tag and tags the meeting (Enter)", async () => {
    commands.createTag.mockReturnValue(ok(tag("t9", "Review")));
    renderLive(<FolderTags meeting="m1" />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Add tag…" }));
    await user.type(screen.getByRole("combobox", { name: "Tag name" }), "Review{Enter}");
    await waitFor(() => expect(commands.createTag).toHaveBeenCalledExactlyOnceWith("Review"));
    await waitFor(() => expect(commands.tagMeetings).toHaveBeenCalledExactlyOnceWith(["m1"], "t9"));
  });

  it("a refusal from the core is a sentence (limit)", async () => {
    commands.tagMeetings.mockReturnValue(err("limit"));
    renderLive(<FolderTags meeting="m1" />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Add tag…" }));
    await user.type(screen.getByRole("combobox", { name: "Tag name" }), "hop");
    await user.click(await screen.findByRole("option", { name: "Họp" }));
    expect(await screen.findByText("You have reached the limit of 20.")).toBeTruthy();
  });
});

describe("Move to folder / Add tag on a selection", () => {
  it("moves the selected meetings and says how many", async () => {
    renderLive(<BulkOrganize meetings={["m1", "m2"]} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Move to folder…" }));
    await user.click(await screen.findByRole("button", { name: /Clients/ }));
    await waitFor(() => expect(commands.moveToFolder).toHaveBeenCalledExactlyOnceWith(["m1", "m2"], "f1"));
    expect(await screen.findByText("Moved 2 meetings to Clients")).toBeTruthy();
  });

  it("No folder moves them out", async () => {
    renderLive(<BulkOrganize meetings={["m2"]} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Move to folder…" }));
    await user.click(await screen.findByRole("button", { name: "No folder" }));
    await waitFor(() => expect(commands.moveToFolder).toHaveBeenCalledExactlyOnceWith(["m2"], null));
  });

  it("a new folder is made and used; a duplicate name says so", async () => {
    commands.createFolder.mockReturnValueOnce(err("duplicate"));
    renderLive(<BulkOrganize meetings={["m1"]} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Move to folder…" }));
    const input = await screen.findByRole("textbox", { name: "Folder name" });
    await user.type(input, "hop{Enter}");
    expect(await screen.findByText("“hop” already exists.")).toBeTruthy();
    expect(commands.moveToFolder).not.toHaveBeenCalled();
    commands.createFolder.mockReturnValue(ok(folder("f2", "Hop")));
    await user.clear(input);
    await user.type(input, "Hop{Enter}");
    await waitFor(() => expect(commands.moveToFolder).toHaveBeenCalledExactlyOnceWith(["m1"], "f2"));
  });

  it("Add tag… tags every selected meeting", async () => {
    renderLive(<BulkOrganize meetings={["m1", "m2"]} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Add tag…" }));
    await user.type(await screen.findByRole("combobox", { name: "Tag name" }), "ho");
    await user.click(await screen.findByRole("option", { name: "Họp" }));
    await waitFor(() => expect(commands.tagMeetings).toHaveBeenCalledExactlyOnceWith(["m1", "m2"], "t1"));
  });
});

describe("Sidebar folders", () => {
  it("lists folders with counts as links, and makes a new one", async () => {
    commands.createFolder.mockReturnValue(ok(folder("f2", "Ops")));
    renderLive(<SidebarFolders />);
    const link = await screen.findByRole("link", { name: /Clients/ });
    expect(link.getAttribute("href")).toContain("folder=f1");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "New folder…" }));
    await user.type(screen.getByRole("textbox", { name: "Folder name" }), "Ops{Enter}");
    await waitFor(() => expect(commands.createFolder).toHaveBeenCalledExactlyOnceWith("Ops"));
  });

  it("a double Enter makes the folder once", async () => {
    let resolve!: (v: unknown) => void;
    commands.createFolder.mockReturnValue(new Promise((r) => (resolve = r)));
    renderLive(<SidebarFolders />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "New folder…" }));
    await user.type(screen.getByRole("textbox", { name: "Folder name" }), "Ops{Enter}{Enter}");
    resolve({ status: "ok", data: folder("f2", "Ops") });
    await waitFor(() => expect(commands.createFolder).toHaveBeenCalledTimes(1));
  });
});

describe("Settings → Folders and tags", () => {
  it("renames a folder", async () => {
    renderLive(<OrganizeCard />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Rename folder: Clients" }));
    const input = screen.getByRole("textbox", { name: "Rename folder" });
    await user.clear(input);
    await user.type(input, "Customers{Enter}");
    await waitFor(() => expect(commands.renameFolder).toHaveBeenCalledExactlyOnceWith("f1", "Customers"));
  });

  it("deleting asks first and says the meetings stay", async () => {
    renderLive(<OrganizeCard />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Delete folder: Clients" }));
    const ask = screen.getByRole("alertdialog");
    expect(ask.textContent).toContain("Its 2 meetings stay in Meetings.");
    expect(commands.deleteFolder).not.toHaveBeenCalled();
    await user.click(within(ask).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(commands.deleteFolder).toHaveBeenCalledExactlyOnceWith("f1"));
    expect(commands.deleteTag).not.toHaveBeenCalled();
  });

  it("deleting a tag says how many meetings lose it", async () => {
    renderLive(<OrganizeCard />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Delete tag: Họp" }));
    expect(screen.getByRole("alertdialog").textContent).toContain("It is removed from 3 meetings.");
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(commands.deleteTag).toHaveBeenCalledExactlyOnceWith("t1"));
  });

  it("a duplicate name when renaming is a sentence", async () => {
    commands.renameTag.mockReturnValue(err("duplicate"));
    renderLive(<OrganizeCard />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Rename tag: Họp" }));
    const input = screen.getByRole("textbox", { name: "Rename tag" });
    await user.clear(input);
    await user.type(input, "Hộp{Enter}");
    expect(await screen.findByText("“Hộp” already exists.")).toBeTruthy();
  });

  it("an empty state when there is nothing", async () => {
    folders = [];
    tags = [];
    renderLive(<OrganizeCard />);
    expect(await screen.findByText(/No folders or tags yet/)).toBeTruthy();
  });
});

// fireEvent is used for the IME case below.
describe("tag combobox", () => {
  it("does not pick while an IME composition is confirmed with Enter", async () => {
    renderLive(<FolderTags meeting="m1" />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Add tag…" }));
    const box = screen.getByRole("combobox", { name: "Tag name" });
    fireEvent.change(box, { target: { value: "hop" } });
    fireEvent.keyDown(box, { key: "Enter", isComposing: true });
    expect(commands.tagMeetings).not.toHaveBeenCalled();
  });
});
