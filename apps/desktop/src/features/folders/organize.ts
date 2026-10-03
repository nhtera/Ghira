// SPDX-License-Identifier: Apache-2.0
// Folders and tags (phase 14d, D7): the reads, the commands with their toasts,
// and the name rules the UI shares with the core. Names compare ignoring case
// but NOT accents ("Họp" and "Hộp" are different); suggestions fold accents so
// typing "hop" finds both.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { TFunction } from "i18next";
import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import type { FolderRow, TagRow } from "../../bindings";
import { ipc } from "../../ipc";
import { MEETINGS_KEY } from "../library/use-meetings";

/** The core's limits (the core enforces them; these only word the messages). */
export const MAX = { folderName: 60, tagName: 40, folders: 200, tags: 200, tagsPerMeeting: 20 } as const;

export const foldersKey = ["folders"] as const;
export const tagsKey = ["tags"] as const;

const unwrap = async <T>(p: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>): Promise<T> => {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
};

export const useFolders = () => useQuery({ queryKey: foldersKey, queryFn: (): Promise<FolderRow[]> => unwrap(ipc.commands.listFolders()) });
export const useTags = () => useQuery({ queryKey: tagsKey, queryFn: (): Promise<TagRow[]> => unwrap(ipc.commands.listTags()) });

/** The same name as the core sees it: trimmed, NFC, lower case, accents kept. */
export const nameKey = (s: string) => s.trim().replace(/\s+/g, " ").normalize("NFC").toLowerCase();

/** Whether the text has accents (the core reuses a tag, or refuses a folder, as an accent variant only when it has none). */
export const hasAccents = (s: string) => foldName(s) !== s.trim().normalize("NFC").toLowerCase();

/** For suggestions only: accents and case ignored ("hop" matches "Họp"). */
export const foldName = (s: string) =>
  s
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[đĐ]/g, "d")
    .toLowerCase()
    .trim();

/** What changes when folders, tags or assignments change. */
export function useInvalidateOrganize() {
  const client = useQueryClient();
  return useCallback(() => {
    for (const queryKey of [foldersKey, tagsKey, MEETINGS_KEY, ["search"], ["related"]] as const) void client.invalidateQueries({ queryKey });
  }, [client]);
}

/** A refusal code as a sentence. `empty` has none: the UI never sends an empty name. */
export function organizeError(t: TFunction, code: string, ctx: { name?: string; max?: number } = {}): string | null {
  switch (code) {
    case "duplicate":
      return t("organize.duplicate", { name: ctx.name ?? "" });
    case "tooLong":
      return t("organize.tooLong", { max: ctx.max ?? MAX.tagName });
    case "limit":
      return t("organize.limit", { max: ctx.max ?? MAX.tagsPerMeeting });
    case "empty":
      return null;
    case "notFound":
      return t("organize.notFound");
    default:
      // `storage` and anything unknown: never show the raw code.
      return t("organize.failed");
  }
}

/** The organize commands with their toasts. Each returns whether it worked (or the row it made). */
export function useOrganizeActions() {
  const { t } = useTranslation();
  const { show } = useToast();
  const invalidate = useInvalidateOrganize();
  return useMemo(() => {
    const warn = (code: string, ctx?: { name?: string; max?: number }) => {
      const title = organizeError(t, code, ctx);
      if (title) show({ tone: "warning", title });
    };
    return {
      async createFolder(name: string): Promise<FolderRow | null> {
        const r = await ipc.commands.createFolder(name);
        if (r.status === "error") return warn(r.error, { name: name.trim(), max: MAX.folderName }), null;
        invalidate();
        return r.data;
      },
      async createTag(name: string): Promise<TagRow | null> {
        const r = await ipc.commands.createTag(name);
        if (r.status === "error") return warn(r.error, { name: name.trim(), max: r.error === "limit" ? MAX.tags : MAX.tagName }), null;
        invalidate();
        return r.data;
      },
      async move(meetings: string[], folder: FolderRow | null): Promise<boolean> {
        const r = await ipc.commands.moveToFolder(meetings, folder?.gid ?? null);
        if (r.status === "error") return warn(r.error), false;
        invalidate();
        show({ tone: "success", title: t("organize.moved", { count: meetings.length, folder: folder?.name ?? t("organize.noFolder") }) });
        return true;
      },
      async tag(meetings: string[], tag: TagRow): Promise<boolean> {
        const r = await ipc.commands.tagMeetings(meetings, tag.gid);
        if (r.status === "error") return warn(r.error, { max: MAX.tagsPerMeeting }), false;
        invalidate();
        return true;
      },
      async untag(meetings: string[], tag: { gid: string }): Promise<boolean> {
        const r = await ipc.commands.untagMeetings(meetings, tag.gid);
        if (r.status === "error") return warn(r.error), false;
        invalidate();
        return true;
      },
    };
  }, [invalidate, show, t]);
}
