// SPDX-License-Identifier: Apache-2.0
// The mock core's folders and tags (phase 14d, D7): real state over the
// library rows (`folder`, `tags` on each row), with the real limits and
// error codes. W0-B baseline; slice S6 extends it.
//   ?organizefail=<code>   every folder/tag command fails with that code
import type { FolderRow, MeetingRow, TagRow } from "../bindings";
import type { Commands } from "./ipc";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> => Promise.resolve({ status: "error", error });
const flag = (name: string) => new URLSearchParams(location.search).get(name);

const FOLDER_MAX = 60;
const TAG_MAX = 40;
const TAGS_PER_MEETING = 20;
const key = (name: string) => name.trim().replace(/\s+/g, " ").normalize("NFC").toLowerCase();
/** Accents and case ignored, only to find a lone accent variant ("hop" for "Họp"), as the store does. */
const fold = (name: string) =>
  name
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[đĐ]/g, "d")
    .toLowerCase()
    .trim();
const hasAccents = (name: string) => fold(name) !== name.trim().normalize("NFC").toLowerCase();
/** The one entry whose name differs from `name` only by accents (none or several: undefined). */
const loneVariant = <T extends { name: string }>(list: T[], name: string): T | undefined => {
  const same = list.filter((x) => fold(x.name) === fold(name));
  return same.length === 1 ? same[0] : undefined;
};

export interface OrganizeHost {
  rows: MeetingRow[];
}

type OrganizeCommands = Pick<
  Commands,
  | "listFolders"
  | "createFolder"
  | "renameFolder"
  | "deleteFolder"
  | "moveToFolder"
  | "listTags"
  | "createTag"
  | "renameTag"
  | "deleteTag"
  | "tagMeetings"
  | "untagMeetings"
>;

export function organizeCommands(host: OrganizeHost): OrganizeCommands {
  let n = 0;
  const folders: { gid: string; name: string }[] = [];
  const tags: { gid: string; name: string }[] = [];
  const refused = () => flag("organizefail");
  const folderRow = (f: { gid: string; name: string }): FolderRow => ({ ...f, meetings: host.rows.filter((r) => r.folder === f.gid).length });
  const tagRow = (t: { gid: string; name: string }): TagRow => ({ ...t, meetings: host.rows.filter((r) => r.tags.some((x) => x.gid === t.gid)).length });
  const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name);
  const clean = (name: string, max: number) => {
    const v = name.trim();
    return v === "" ? "empty" : [...v].length > max ? "tooLong" : null;
  };

  return {
    listFolders: () => (refused() ? fail(refused()!) : ok([...folders].sort(byName).map(folderRow))),
    createFolder: (name) => {
      if (refused()) return fail(refused()!);
      const bad = clean(name, FOLDER_MAX);
      if (bad) return fail(bad);
      // The core also refuses a name that differs from a lone folder only by accents.
      // (Only for a name typed without accents: "Hộp" is allowed beside "Họp".)
      if (folders.some((f) => key(f.name) === key(name)) || (!hasAccents(name) && loneVariant(folders, name))) return fail("duplicate");
      const f = { gid: `folder-${++n}`, name: name.trim() };
      folders.push(f);
      return ok(folderRow(f));
    },
    renameFolder: (gid, name) => {
      if (refused()) return fail(refused()!);
      const f = folders.find((x) => x.gid === gid);
      if (!f) return fail("notFound");
      const bad = clean(name, FOLDER_MAX);
      if (bad) return fail(bad);
      if (folders.some((x) => x.gid !== gid && key(x.name) === key(name))) return fail("duplicate");
      f.name = name.trim();
      return ok(null);
    },
    deleteFolder: (gid) => {
      if (refused()) return fail(refused()!);
      const i = folders.findIndex((x) => x.gid === gid);
      if (i < 0) return fail("notFound");
      const inside = host.rows.filter((r) => r.folder === gid);
      inside.forEach((r) => (r.folder = null));
      folders.splice(i, 1);
      return ok(inside.length);
    },
    moveToFolder: (meetings, folder) => {
      if (refused()) return fail(refused()!);
      if (folder != null && !folders.some((f) => f.gid === folder)) return fail("notFound");
      let changed = 0;
      for (const r of host.rows.filter((x) => meetings.includes(x.gid))) {
        if (r.folder !== folder) changed += 1;
        r.folder = folder;
      }
      return ok(changed);
    },
    listTags: () => (refused() ? fail(refused()!) : ok([...tags].sort(byName).map(tagRow))),
    createTag: (name) => {
      if (refused()) return fail(refused()!);
      const bad = clean(name, TAG_MAX);
      if (bad) return fail(bad);
      // The same name, or a lone accent variant ("hop" gives "Họp"), is reused.
      const have = tags.find((t) => key(t.name) === key(name)) ?? (hasAccents(name) ? undefined : loneVariant(tags, name));
      if (have) return ok(tagRow(have));
      const t = { gid: `tag-${++n}`, name: name.trim() };
      tags.push(t);
      return ok(tagRow(t));
    },
    renameTag: (gid, name) => {
      if (refused()) return fail(refused()!);
      const t = tags.find((x) => x.gid === gid);
      if (!t) return fail("notFound");
      const bad = clean(name, TAG_MAX);
      if (bad) return fail(bad);
      if (tags.some((x) => x.gid !== gid && key(x.name) === key(name))) return fail("duplicate");
      t.name = name.trim();
      host.rows.forEach((r) => r.tags.forEach((x) => x.gid === gid && (x.name = t.name)));
      return ok(null);
    },
    deleteTag: (gid) => {
      if (refused()) return fail(refused()!);
      const i = tags.findIndex((x) => x.gid === gid);
      if (i < 0) return fail("notFound");
      let had = 0;
      for (const r of host.rows) {
        if (r.tags.some((x) => x.gid === gid)) had += 1;
        r.tags = r.tags.filter((x) => x.gid !== gid);
      }
      tags.splice(i, 1);
      return ok(had);
    },
    tagMeetings: (meetings, gid) => {
      if (refused()) return fail(refused()!);
      const t = tags.find((x) => x.gid === gid);
      if (!t) return fail("notFound");
      let added = 0;
      for (const r of host.rows.filter((x) => meetings.includes(x.gid))) {
        if (r.tags.some((x) => x.gid === gid)) continue;
        if (r.tags.length >= TAGS_PER_MEETING) return fail("limit");
        r.tags = [...r.tags, { gid: t.gid, name: t.name }];
        added += 1;
      }
      return ok(added);
    },
    untagMeetings: (meetings, gid) => {
      if (refused()) return fail(refused()!);
      if (!tags.some((x) => x.gid === gid)) return fail("notFound");
      let removed = 0;
      for (const r of host.rows.filter((x) => meetings.includes(x.gid))) {
        if (r.tags.some((x) => x.gid === gid)) removed += 1;
        r.tags = r.tags.filter((x) => x.gid !== gid);
      }
      return ok(removed);
    },
  };
}
